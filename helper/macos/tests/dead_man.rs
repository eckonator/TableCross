//! Runs the real helper (without touching keyboards) and checks that it gives the keyboards back
//! when the app falls silent or disconnects.

#![cfg(target_os = "macos")]

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tc_input::helper_protocol::{FromHelper, HELPER_VERSION, ToHelper, read_frame, write_frame};

struct Helper(Child);

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(socket: &str) -> Helper {
    let child = Command::new(env!("CARGO_BIN_EXE_tablecross-helper"))
        .env("TC_HELPER_SOCKET", socket)
        .env("TC_HELPER_DRY_RUN", "1")
        .spawn()
        .unwrap();
    Helper(child)
}

fn connect(socket: &str) -> (UnixStream, BufReader<UnixStream>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let reader = BufReader::new(stream.try_clone().unwrap());
            return (stream, reader);
        }
        assert!(Instant::now() < deadline, "helper did not open its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn next_status(reader: &mut BufReader<UnixStream>) -> bool {
    loop {
        match read_frame::<FromHelper>(reader).unwrap() {
            FromHelper::Status { seized, .. } => return seized,
            _ => continue,
        }
    }
}

#[test]
fn keyboards_are_given_back_when_the_app_goes_quiet_or_away() {
    let socket = std::env::temp_dir().join(format!("tablecross-helper-test-{}.sock", std::process::id()));
    let socket = socket.to_str().unwrap().to_string();
    let _helper = start(&socket);
    let (mut stream, mut reader) = connect(&socket);
    write_frame(
        &mut stream,
        &ToHelper::Hello {
            version: HELPER_VERSION,
        },
    )
    .unwrap();
    assert_eq!(
        read_frame::<FromHelper>(&mut reader).unwrap(),
        FromHelper::Hello {
            version: HELPER_VERSION,
            access: true
        }
    );
    assert!(!next_status(&mut reader));

    write_frame(&mut stream, &ToHelper::Seize(true)).unwrap();
    assert!(next_status(&mut reader));
    // Silence: given back after about a second
    let silent = Instant::now();
    assert!(!next_status(&mut reader));
    let after = silent.elapsed();
    assert!(
        after >= Duration::from_millis(900) && after < Duration::from_secs(2),
        "{after:?}"
    );

    // Signs of life bring them back, closing the connection gives them back at once
    write_frame(&mut stream, &ToHelper::Alive).unwrap();
    assert!(next_status(&mut reader));
    let (mut second, mut second_reader) = connect(&socket);
    write_frame(
        &mut second,
        &ToHelper::Hello {
            version: HELPER_VERSION,
        },
    )
    .unwrap();
    // A new connection replaces the old one and starts with the keyboards given back
    let mut saw_release = false;
    for _ in 0..4 {
        match read_frame::<FromHelper>(&mut second_reader).unwrap() {
            FromHelper::Status { seized: false, .. } => {
                saw_release = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw_release);
    drop(stream);
}
