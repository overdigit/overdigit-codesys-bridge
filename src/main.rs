use inotify::{EventMask, Inotify, WatchMask};
use std::error::Error;
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

mod bridge;

// Credit: https://github.com/nroi/waitforfile/blob/master/src/main.rs
fn wait_for(dirname: &std::path::Path) -> Result<bool, Box<dyn Error>> {
    let mut ino = Inotify::init()?;
    let parent = dirname.parent().expect("This folder has no parent. How? It's the default folder, I wrote its path. What did you do?");
    ino.watches()
        .add(parent, WatchMask::DELETE_SELF | WatchMask::CREATE)
        .unwrap();
    if !dirname.exists() {
        loop {
            let mut buffer = [0; 1024];
            let events = ino
                .read_events_blocking(&mut buffer)
                .expect("Error while reading events");
            for event in events {
                match event.name {
                    Some(name) => {
                        if parent.join(name) == dirname {
                            return Ok(true);
                        }
                    }
                    None => {
                        if event.mask == EventMask::DELETE_SELF {
                            return Err("The watched directory has been deleted.".into());
                        }
                    }
                }
            }
        }
    }
    // file already exists prior to running this program.
    Ok(false)
}

fn get_uid(username: &str) -> std::io::Result<Option<u32>> {
    Ok(std::fs::read_to_string("/etc/passwd")?
        .lines()
        .find_map(|l| {
            let v = l.split(':').collect::<Vec<_>>();
            if v.len() == 7 && v.first() == Some(&username) {
                return v.get(2)?.parse::<u32>().ok();
            }
            None
        }))
}

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
        wait_for(path)?;
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
    let socket_uid = get_uid(&socket_user)?.unwrap_or_else(|| {
        panic!(
            "{}",
            format!("Could not find user {socket_user} in /etc/passwd.")
        )
    });

    let listener = UnixListener::bind(&sock_path)?;

    std::os::unix::fs::chown(&sock_path, Some(socket_uid), None)?;

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
