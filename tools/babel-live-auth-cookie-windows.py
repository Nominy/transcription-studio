#!/usr/bin/env python3
import argparse
import base64
import ctypes
import json
import os
import shutil
import sqlite3
import sys
import tempfile
from ctypes import wintypes
from pathlib import Path

from cryptography.hazmat.primitives.ciphers.aead import AESGCM


DEFAULT_BABEL_URL = "https://dashboard.babel.audio"


class DATA_BLOB(ctypes.Structure):
    _fields_ = [
        ("cbData", wintypes.DWORD),
        ("pbData", ctypes.POINTER(ctypes.c_char)),
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default=os.environ.get("STUDIO_BABEL_SERVICE_BASE_URL", DEFAULT_BABEL_URL))
    parser.add_argument("--user-data-dir", default=os.environ.get("STUDIO_BABEL_CHROME_USER_DATA_DIR"))
    args = parser.parse_args()

    user_data_dir = Path(args.user_data_dir) if args.user_data_dir else default_user_data_dir()
    if not user_data_dir.exists():
        raise RuntimeError(f"Chrome user data dir does not exist: {user_data_dir}")

    host = host_from_url(args.url)
    key = chrome_aes_key(user_data_dir)
    cookies = []
    errors = []
    for db_path in cookie_db_paths(user_data_dir):
        try:
            cookies.extend(read_cookies_from_db(db_path, host, key))
        except Exception as exc:
            errors.append(f"{db_path}: {exc}")

    deduped = {}
    for name, value in cookies:
        if value:
            deduped[name] = value

    if not deduped:
        detail = "; ".join(errors[:3])
        raise RuntimeError(f"No decryptable cookies found for {host}" + (f" ({detail})" if detail else ""))

    print("; ".join(f"{name}={value}" for name, value in deduped.items()))
    return 0


def default_user_data_dir() -> Path:
    local_app_data = os.environ.get("LOCALAPPDATA")
    if not local_app_data:
        raise RuntimeError("LOCALAPPDATA is not set")
    return Path(local_app_data) / "Google" / "Chrome" / "User Data"


def host_from_url(url: str) -> str:
    from urllib.parse import urlparse

    host = urlparse(url).hostname
    if not host:
        raise RuntimeError(f"Could not parse host from URL: {url}")
    return host


def chrome_aes_key(user_data_dir: Path) -> bytes:
    local_state = json.loads((user_data_dir / "Local State").read_text(encoding="utf-8"))
    encrypted_key = base64.b64decode(local_state["os_crypt"]["encrypted_key"])
    if encrypted_key.startswith(b"DPAPI"):
        encrypted_key = encrypted_key[5:]
    return dpapi_unprotect(encrypted_key)


def cookie_db_paths(user_data_dir: Path):
    profiles = [user_data_dir / "Default"]
    profiles.extend(sorted(user_data_dir.glob("Profile *")))
    for profile in profiles:
        network_db = profile / "Network" / "Cookies"
        legacy_db = profile / "Cookies"
        if network_db.exists():
            yield network_db
        if legacy_db.exists():
            yield legacy_db


def read_cookies_from_db(db_path: Path, host: str, key: bytes):
    temp_dir = Path(tempfile.mkdtemp(prefix="babel-cookie-db-"))
    temp_db = temp_dir / "Cookies"
    try:
        copy_file_shared(db_path, temp_db)
        connection = sqlite3.connect(temp_db)
        try:
            rows = connection.execute(
                """
                SELECT host_key, name, value, encrypted_value
                FROM cookies
                WHERE host_key = ? OR host_key = ? OR host_key LIKE ?
                """,
                (host, "." + host, "%." + host),
            ).fetchall()
        finally:
            connection.close()

        cookies = []
        for _host_key, name, value, encrypted_value in rows:
            decrypted = value or decrypt_cookie_value(bytes(encrypted_value), key)
            if decrypted:
                cookies.append((name, decrypted))
        return cookies
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def decrypt_cookie_value(encrypted_value: bytes, key: bytes) -> str:
    if not encrypted_value:
        return ""
    if encrypted_value.startswith((b"v10", b"v11", b"v20")):
        nonce = encrypted_value[3:15]
        ciphertext_and_tag = encrypted_value[15:]
        plaintext = AESGCM(key).decrypt(nonce, ciphertext_and_tag, None)
        return plaintext.decode("utf-8")
    return dpapi_unprotect(encrypted_value).decode("utf-8")


def copy_file_shared(source: Path, target: Path) -> None:
    if os.name != "nt":
        shutil.copy2(source, target)
        return

    kernel32 = ctypes.windll.kernel32
    generic_read = 0x80000000
    file_share_read = 0x00000001
    file_share_write = 0x00000002
    file_share_delete = 0x00000004
    open_existing = 3
    normal_attributes = 0x00000080
    invalid_handle = ctypes.c_void_p(-1).value

    kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.ReadFile.argtypes = [
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        wintypes.LPVOID,
    ]
    kernel32.ReadFile.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    handle = kernel32.CreateFileW(
        str(source),
        generic_read,
        file_share_read | file_share_write | file_share_delete,
        None,
        open_existing,
        normal_attributes,
        None,
    )
    if handle == invalid_handle:
        raise ctypes.WinError()

    try:
        buffer_size = 1024 * 1024
        buffer = ctypes.create_string_buffer(buffer_size)
        bytes_read = wintypes.DWORD(0)
        with open(target, "wb") as output:
            while True:
                ok = kernel32.ReadFile(
                    handle,
                    buffer,
                    buffer_size,
                    ctypes.byref(bytes_read),
                    None,
                )
                if not ok:
                    raise ctypes.WinError()
                if bytes_read.value == 0:
                    break
                output.write(buffer.raw[: bytes_read.value])
    finally:
        kernel32.CloseHandle(handle)


def dpapi_unprotect(data: bytes) -> bytes:
    crypt32 = ctypes.windll.crypt32
    kernel32 = ctypes.windll.kernel32

    input_blob = DATA_BLOB(len(data), ctypes.cast(ctypes.create_string_buffer(data), ctypes.POINTER(ctypes.c_char)))
    output_blob = DATA_BLOB()

    if not crypt32.CryptUnprotectData(
        ctypes.byref(input_blob),
        None,
        None,
        None,
        None,
        0,
        ctypes.byref(output_blob),
    ):
        raise ctypes.WinError()

    try:
        return ctypes.string_at(output_blob.pbData, output_blob.cbData)
    finally:
        kernel32.LocalFree(output_blob.pbData)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"babel-live-auth-cookie-windows: {exc}", file=sys.stderr)
        raise SystemExit(1)
