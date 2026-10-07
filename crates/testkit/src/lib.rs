#![forbid(unsafe_code)]

pub mod clock;
pub mod daemon;
pub mod temp;
pub mod types;

use std::fmt::Debug;
use std::time::Duration;

pub const FACT_LIVENESS: Duration = Duration::from_secs(30);

pub fn codec_laws<T, EncodeError, DecodeError>(
    encode: impl Fn(&T) -> Result<Vec<u8>, EncodeError>,
    decode: impl Fn(&[u8]) -> Result<T, DecodeError>,
    golden: &[&[u8]],
    rejected: &[&[u8]],
) where
    T: Debug,
    EncodeError: Debug,
    DecodeError: Debug,
{
    assert!(
        !golden.is_empty(),
        "without golden bytes the first law is vacuously true"
    );
    for (index, bytes) in golden.iter().enumerate() {
        let value = decode(bytes)
            .unwrap_or_else(|error| panic!("golden {index} does not decode: {error:?}"));
        let again = encode(&value).unwrap_or_else(|error| {
            panic!("golden {index} value {value:?} does not encode: {error:?}")
        });
        assert_eq!(
            again.as_slice(),
            *bytes,
            "golden {index} came back as different bytes; one value has two spellings"
        );
    }
    for (index, bytes) in rejected.iter().enumerate() {
        if let Ok(value) = decode(bytes) {
            panic!("refusal {index} decoded to the value {value:?}");
        }
    }
}
