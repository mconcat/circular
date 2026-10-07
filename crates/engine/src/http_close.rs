
use std::io::Read;
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

pub fn close_after_response(stream: &mut TcpStream, byte_wall: usize, io_timeout: Duration) {
    if stream.shutdown(Shutdown::Write).is_err()
        || stream.set_read_timeout(Some(io_timeout)).is_err()
    {
        return;
    }
    discard_until_end(stream, byte_wall, io_timeout);
}

fn discard_until_end(stream: &mut impl Read, byte_wall: usize, deadline: Duration) {
    let started = Instant::now();
    let mut remaining = byte_wall;
    let mut buffer = [0_u8; 4096];
    while remaining > 0 && started.elapsed() < deadline {
        let length = remaining.min(buffer.len());
        match stream.read(&mut buffer[..length]) {
            Ok(0) | Err(_) => return,
            Ok(read) => remaining -= read,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn discarding_stops_at_the_byte_wall() {
        let mut stream = Cursor::new(vec![b'x'; 2 * 1024 * 1024]);
        discard_until_end(&mut stream, 1024 * 1024, Duration::from_secs(5));
        assert_eq!(stream.position(), 1024 * 1024);
    }

    #[test]
    fn discarding_stops_at_the_end_of_the_peer() {
        let mut stream = Cursor::new(vec![b'x'; 10]);
        discard_until_end(&mut stream, 1024 * 1024, Duration::from_secs(5));
        assert_eq!(stream.position(), 10);
    }
}
