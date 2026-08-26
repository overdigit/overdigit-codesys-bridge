use std::error::Error;
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

mod bridge;
mod util;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let dir_env = std::env::var("WP360_CODESYS_SOCK_DIR");
    let env_set = dir_env.is_ok();
    let dir = dir_env.unwrap_or("/var/opt/codesyscontrolapi/extfuncs/".to_string());

    let dir_path = std::path::Path::new(&dir);
    if !dir_path.is_dir() {
        if env_set {
            return Err(
                format!("Invalid path - \"{dir}\" does not exist or is not a directory").into(),
            );
        }
        let path = std::path::Path::new(&dir);
        util::wait_for(path)?;
    }
    let sock_path = dir_path.join("wp360-bridge.sock");
    match sock_path.try_exists() {
        Ok(true) => {
            match UnixStream::connect(&sock_path) {
                Ok(stream) => {
                    // Socket is in use
                    stream.shutdown(std::net::Shutdown::Both)?;
                    // Gonna let the ::bind call throw an error instead
                }
                Err(_e) => {
                    // Socket is not in use or at the very least not connectable, safe to remove
                    std::fs::remove_file(&sock_path)?;
                }
            }
        }
        Ok(false) => {}
        Err(e) => return Err(Box::new(e)),
    }

    let socket_user =
        std::env::var("WP360_CODESYS_SOCK_USER").unwrap_or("codesyscontrol".to_string());
    let socket_uid = util::get_uid(&socket_user)?.unwrap_or_else(|| {
        panic!(
            "{}",
            format!("Could not find user {socket_user} in /etc/passwd.")
        )
    });

    let listener = UnixListener::bind(&sock_path)?;

    std::os::unix::fs::chown(&sock_path, Some(socket_uid), None)?;
    std::fs::set_permissions(&sock_path, Permissions::from_mode(0o775))?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| bridge::client(stream));
            }
            Err(err) => {
                std::fs::remove_file(&sock_path)?;
                return Err(Box::new(err));
            }
        }
    }
    Ok(())
}
