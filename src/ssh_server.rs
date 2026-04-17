use crate::pty;
use crate::server::sanitize_name;
use anyhow::Result;
use async_trait::async_trait;
use russh::server::{Auth, Config, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, Pty};
use russh_keys::PrivateKey;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

pub async fn run(port: u16, host_key_path: Option<PathBuf>, socket_path: &str) -> Result<()> {
    let key = load_or_generate_host_key(host_key_path)?;
    let config = Config {
        keys: vec![key],
        inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
        auth_rejection_time: std::time::Duration::from_secs(1),
        ..Default::default()
    };
    let exe = std::env::current_exe()?;
    let mut server = SshServerImpl {
        socket_path: socket_path.to_string(),
        exe_path: exe,
    };
    eprintln!("[delvers ssh] listening on 0.0.0.0:{port}");
    server
        .run_on_address(Arc::new(config), ("0.0.0.0", port))
        .await?;
    Ok(())
}

fn load_or_generate_host_key(path: Option<PathBuf>) -> Result<PrivateKey> {
    let key_path = path.unwrap_or_else(default_key_path);
    if key_path.exists() {
        eprintln!("[delvers ssh] loading host key from {}", key_path.display());
        Ok(russh_keys::load_secret_key(&key_path, None)?)
    } else {
        eprintln!(
            "[delvers ssh] generating new host key at {}",
            key_path.display()
        );
        let key = PrivateKey::random(&mut rand::rngs::OsRng, russh_keys::Algorithm::Ed25519)
            .map_err(|e| anyhow::anyhow!("keygen failed: {e}"))?;
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&key_path)?;
        russh_keys::encode_pkcs8_pem(&key, &mut f)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(key)
    }
}

fn default_key_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set; use --host-key to specify key path");
    PathBuf::from(home).join(".config/delvers/ssh_host_ed25519_key")
}

#[derive(Clone)]
struct SshServerImpl {
    socket_path: String,
    exe_path: PathBuf,
}

impl Server for SshServerImpl {
    type Handler = SshSession;

    fn new_client(&mut self, peer: Option<std::net::SocketAddr>) -> SshSession {
        eprintln!("[delvers ssh] connection from {peer:?}");
        SshSession {
            socket_path: self.socket_path.clone(),
            exe_path: self.exe_path.clone(),
            username: String::new(),
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24,
            channel_id: None,
            child_pid: None,
            pty_writer: None,
            ioctl_fd: None,
        }
    }
}

struct SshSession {
    socket_path: String,
    exe_path: PathBuf,
    username: String,
    term: String,
    cols: u16,
    rows: u16,
    channel_id: Option<ChannelId>,
    child_pid: Option<u32>,
    pty_writer: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    ioctl_fd: Option<OwnedFd>,
}

#[async_trait]
impl Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        self.username = sanitize_name(user);
        Ok(Auth::Accept)
    }

    async fn auth_password(&mut self, user: &str, _password: &str) -> Result<Auth, Self::Error> {
        self.username = sanitize_name(user);
        Ok(Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        _key: &russh_keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.username = sanitize_name(user);
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.channel_id = Some(channel.id());
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term = term.to_string();
        self.cols = col_width as u16;
        self.rows = row_height as u16;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;

        let (master, slave) = pty::open_pty_pair()?;
        pty::set_window_size(master.as_raw_fd(), self.cols, self.rows)?;

        let slave_raw = slave.into_raw_fd();
        let mut cmd = Command::new(&self.exe_path);
        cmd.args([
            "client",
            "--name",
            &self.username,
            "--socket",
            &self.socket_path,
        ]);
        cmd.env("TERM", &self.term);
        unsafe {
            cmd.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::c_ulong::from(libc::TIOCSCTTY), 0i32) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
            cmd.stdin(Stdio::from_raw_fd(libc::dup(slave_raw)));
            cmd.stdout(Stdio::from_raw_fd(libc::dup(slave_raw)));
            cmd.stderr(Stdio::from_raw_fd(slave_raw));
        }

        let child = cmd.spawn()?;
        let child_pid = child.id();
        self.child_pid = Some(child_pid);
        eprintln!(
            "[delvers ssh] spawned client pid={child_pid} for '{}'",
            self.username
        );

        let master_raw = master.as_raw_fd();
        self.ioctl_fd = Some(unsafe { OwnedFd::from_raw_fd(libc::dup(master_raw)) });

        let master_file: std::fs::File = master.into();
        let reader_file = master_file.try_clone()?;
        let writer_file = master_file;

        let (write_tx, write_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut w = writer_file;
            while let Ok(data) = write_rx.recv() {
                if w.write_all(&data).is_err() {
                    break;
                }
            }
        });
        self.pty_writer = Some(write_tx);

        let handle = session.handle();
        let (read_tx, mut read_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        std::thread::spawn(move || {
            use std::io::Read;
            let mut r = reader_file;
            let mut buf = [0u8; 4096];
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if read_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        tokio::spawn(async move {
            while let Some(data) = read_rx.recv().await {
                if handle.data(channel, data.into()).await.is_err() {
                    break;
                }
            }
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });

        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });

        Ok(())
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(ref tx) = self.pty_writer {
            let _ = tx.send(data.to_vec());
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(ref fd) = self.ioctl_fd {
            let _ = pty::set_window_size(fd.as_raw_fd(), col_width as u16, row_height as u16);
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.cleanup();
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.cleanup();
        Ok(())
    }
}

impl SshSession {
    fn cleanup(&mut self) {
        if let Some(pid) = self.child_pid.take() {
            eprintln!("[delvers ssh] cleaning up pid={pid}");
            unsafe {
                libc::kill(pid as i32, libc::SIGHUP);
            }
        }
        self.pty_writer = None;
        self.ioctl_fd = None;
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_generate_creates_and_reloads_key() {
        let dir = std::env::temp_dir().join(format!("delvers-test-{}", std::process::id()));
        let key_path = dir.join("test_host_key");
        let _ = std::fs::remove_dir_all(&dir);

        let key1 = load_or_generate_host_key(Some(key_path.clone())).unwrap();
        assert!(key_path.exists());

        let key2 = load_or_generate_host_key(Some(key_path.clone())).unwrap();
        assert_eq!(
            key1.public_key().to_bytes().unwrap(),
            key2.public_key().to_bytes().unwrap(),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_key_has_restricted_permissions() {
        let dir = std::env::temp_dir().join(format!("delvers-perms-{}", std::process::id()));
        let key_path = dir.join("test_host_key");
        let _ = std::fs::remove_dir_all(&dir);

        load_or_generate_host_key(Some(key_path.clone())).unwrap();

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
