//! 用系统默认浏览器打开 URL（Linux：`xdg-open`）。

use std::io;
use std::process::{Command, Stdio};
use std::thread;

pub fn open_browser(url: &str) -> io::Result<()> {
    let mut child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap without blocking the HTTP loop (`xdg-open` may wait until the browser exits).
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
