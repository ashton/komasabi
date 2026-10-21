//! Runs the reader on its own, outside the app: `komasabi-reader <dir> [--page N]`
//! pages through a folder of images in filename order. See [`komasabi_reader::run`].

fn main() -> iced::Result {
    komasabi_reader::run(std::env::args().skip(1))
}
