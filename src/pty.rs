use anyhow::Result;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

pub fn open_pty_pair() -> Result<(OwnedFd, OwnedFd)> {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret != 0 {
        anyhow::bail!("openpty failed: {}", std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) })
}

pub fn set_window_size(fd: RawFd, cols: u16, rows: u16) -> Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
    if ret != 0 {
        anyhow::bail!(
            "TIOCSWINSZ failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    #[test]
    fn open_pty_pair_returns_valid_fds() {
        let (master, slave) = open_pty_pair().unwrap();
        assert!(master.as_raw_fd() >= 0);
        assert!(slave.as_raw_fd() >= 0);
        assert_ne!(master.as_raw_fd(), slave.as_raw_fd());
    }

    #[test]
    fn pty_pair_is_connected() {
        let (master, slave) = open_pty_pair().unwrap();
        let mut master_f: std::fs::File = master.into();
        let mut slave_f: std::fs::File = slave.into();

        slave_f.write_all(b"hello").unwrap();
        slave_f.flush().unwrap();

        let mut buf = [0u8; 64];
        let n = master_f.read(&mut buf).unwrap();
        assert!(n > 0);
        assert!(std::str::from_utf8(&buf[..n]).unwrap().contains("hello"));
    }

    #[test]
    fn set_window_size_succeeds_on_pty() {
        let (master, _slave) = open_pty_pair().unwrap();
        set_window_size(master.as_raw_fd(), 120, 40).unwrap();

        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let ret = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
        assert_eq!(ret, 0);
        assert_eq!(ws.ws_col, 120);
        assert_eq!(ws.ws_row, 40);
    }
}
