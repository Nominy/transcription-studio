use std::{fs, path::PathBuf};

use iced::{Task, window};

use super::Message;

pub(super) fn renderer_screenshot_task(path: PathBuf) -> Task<Message> {
    window::get_latest().and_then(move |id| {
        let path = path.clone();
        window::screenshot(id).map(move |screenshot| {
            Message::RendererSmokeScreenshotSaved(save_renderer_screenshot_bmp(
                &screenshot,
                path.clone(),
            ))
        })
    })
}

fn save_renderer_screenshot_bmp(
    screenshot: &window::Screenshot,
    path: PathBuf,
) -> Result<String, String> {
    let width = screenshot.size.width as usize;
    let height = screenshot.size.height as usize;
    let rgba = screenshot.bytes.as_ref();
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "screenshot dimensions overflowed".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "unexpected screenshot byte length: got {}, expected {expected_len}",
            rgba.len()
        ));
    }

    let row_stride = (width * 3).next_multiple_of(4);
    let pixel_data_len = row_stride
        .checked_mul(height)
        .ok_or_else(|| "BMP pixel data length overflowed".to_string())?;
    let file_size = 54usize
        .checked_add(pixel_data_len)
        .ok_or_else(|| "BMP file size overflowed".to_string())?;
    let mut bytes = Vec::with_capacity(file_size);

    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&(54u32).to_le_bytes());
    bytes.extend_from_slice(&(40u32).to_le_bytes());
    bytes.extend_from_slice(&(width as i32).to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&(1u16).to_le_bytes());
    bytes.extend_from_slice(&(24u16).to_le_bytes());
    bytes.extend_from_slice(&(0u32).to_le_bytes());
    bytes.extend_from_slice(&(pixel_data_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(2_835i32).to_le_bytes());
    bytes.extend_from_slice(&(2_835i32).to_le_bytes());
    bytes.extend_from_slice(&(0u32).to_le_bytes());
    bytes.extend_from_slice(&(0u32).to_le_bytes());

    let padding = vec![0u8; row_stride - width * 3];
    for y in (0..height).rev() {
        for x in 0..width {
            let index = (y * width + x) * 4;
            bytes.extend_from_slice(&[rgba[index + 2], rgba[index + 1], rgba[index]]);
        }
        bytes.extend_from_slice(&padding);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}
