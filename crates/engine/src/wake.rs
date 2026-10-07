
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Instant;

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

#[derive(Debug)]
pub struct Wake {
    read: OwnedFd,
    write: OwnedFd,
}

impl Wake {
    pub fn new() -> io::Result<Self> {
        let (read, write) = nix::unistd::pipe()?;
        for end in [&read, &write] {
            let flags = OFlag::from_bits_truncate(fcntl(end, FcntlArg::F_GETFL)?);
            fcntl(end, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        }
        Ok(Self { read, write })
    }

    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }

    #[must_use]
    pub fn notify_fd(&self) -> BorrowedFd<'_> {
        self.write.as_fd()
    }

    pub fn notify(&self) {
        let _ = nix::unistd::write(&self.write, &[0_u8]);
    }

    pub fn drain(&self) {
        let mut sink = [0_u8; 64];
        while let Ok(read) = nix::unistd::read(&self.read, &mut sink) {
            if read == 0 {
                break;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Woke(u64);

impl Woke {
    #[must_use]
    pub const fn at(self, index: usize) -> bool {
        index < 64 && self.0 & (1 << index) != 0
    }
}

pub fn wait_readable(fds: &[BorrowedFd<'_>]) -> io::Result<Woke> {
    wait_readable_until(fds, None)
}

pub fn wait_readable_until(fds: &[BorrowedFd<'_>], deadline: Option<Instant>) -> io::Result<Woke> {
    let timeout = deadline.map_or(PollTimeout::NONE, |deadline| {
        let left = deadline.saturating_duration_since(Instant::now());
        PollTimeout::try_from(left.as_nanos().div_ceil(1_000_000)).unwrap_or(PollTimeout::MAX)
    });
    let mut polled = fds
        .iter()
        .map(|fd| PollFd::new(*fd, PollFlags::POLLIN))
        .collect::<Vec<_>>();
    match poll(&mut polled, timeout) {
        Ok(_) => {}
        Err(nix::errno::Errno::EINTR) => return Ok(Woke::default()),
        Err(error) => return Err(io::Error::from(error)),
    }
    let mut woke = 0_u64;
    for (index, fd) in polled.iter().enumerate().take(64) {
        if fd.any().unwrap_or(false) {
            woke |= 1 << index;
        }
    }
    Ok(Woke(woke))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_notifications_end_one_wait() {
        let wake = Wake::new().unwrap();
        for _ in 0..1_000 {
            wake.notify();
        }
        wait_readable(&[wake.as_fd()]).unwrap();
        wake.drain();
    }

    #[test]
    fn a_notification_after_a_drain_ends_the_next_wait() {
        let wake = Wake::new().unwrap();
        wake.notify();
        wait_readable(&[wake.as_fd()]).unwrap();
        wake.drain();
        wake.notify();
        wait_readable(&[wake.as_fd()]).unwrap();
        wake.drain();
    }

    #[test]
    fn two_descriptors_are_waited_together_and_the_answer_names_one() {
        let socket = Wake::new().unwrap();
        let wake = Wake::new().unwrap();
        wake.notify();
        let woke = wait_readable(&[socket.as_fd(), wake.as_fd()]).unwrap();
        assert!(!woke.at(0));
        assert!(woke.at(1));
        wake.drain();
        socket.notify();
        let woke = wait_readable(&[socket.as_fd(), wake.as_fd()]).unwrap();
        assert!(woke.at(0));
        assert!(!woke.at(1));
        socket.drain();
    }
}
