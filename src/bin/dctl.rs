//! `dctl` — tiny zero-dependency CLI client for Dullahan's closed-loop dev socket.
//!
//! Usage:
//!   dctl state
//!   dctl screenshot /tmp/view.png
//!   dctl overlay grid
//!   dctl --sock /tmp/custom.sock step 10

use std::io::{BufRead, BufReader, Write};
#[cfg(not(unix))]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

#[cfg(unix)]
const DEFAULT_SOCK: &str = "/tmp/dullahan.sock";
#[cfg(not(unix))]
const DEFAULT_SOCK: &str = "127.0.0.1:42420";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut sock = DEFAULT_SOCK.to_string();
    let mut cmd_parts = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--sock" {
            let Some(p) = args.next() else {
                eprintln!("dctl: --sock requires a path");
                return ExitCode::FAILURE;
            };
            sock = p;
        } else {
            cmd_parts.push(arg);
        }
    }

    if cmd_parts.is_empty() {
        eprintln!("Usage: dctl [--sock <path>] <ping|state|screenshot <path>|pause|resume|step [N]|overlay <mode>|input key <K> [s]|cmd <...>|quit>");
        return ExitCode::FAILURE;
    }

    let line = format!("{}\n", cmd_parts.join(" "));
    #[cfg(unix)]
    let conn = UnixStream::connect(&sock);
    #[cfg(not(unix))]
    let conn = TcpStream::connect(&sock);
    let mut stream = match conn {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dctl: failed to connect to {sock}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("dctl: write failed: {e}");
        return ExitCode::FAILURE;
    }

    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    if let Err(e) = reader.read_line(&mut reply) {
        eprintln!("dctl: read failed: {e}");
        return ExitCode::FAILURE;
    }

    print!("{reply}");
    if reply.starts_with("OK") {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
