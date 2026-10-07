//! Minimal byte-stream surface for local socket adapters.

/// Minimal non-blocking byte-stream surface used by local socket adapters.
/// Implementations may complete either operation with a short count.
pub trait LocalByteStream {
    type Error;

    fn read(&mut self, destination: &mut [u8]) -> Result<usize, Self::Error>;
    fn write(&mut self, source: &[u8]) -> Result<usize, Self::Error>;
}
