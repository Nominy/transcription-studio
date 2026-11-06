fn main() {
    if let Err(error) = exaudio_wasm::worker_main() {
        let message = format!("{error:#}");
        let mut end = message.len().min(4096);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        eprintln!("{}", &message[..end]);
        std::process::exit(1);
    }
}
