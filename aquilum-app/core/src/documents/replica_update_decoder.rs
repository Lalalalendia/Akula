//! Wire-compatible Yrs v1 decoder wrapper for a known malformed-info panic.
//!
//! In Yrs 0.28.0, bit 0x10 is not defined for a block-info byte, but the
//! decoder ignores it. Such a block can be accepted and later fail during
//! re-encoding of pending updates (upstream y-crdt/y-crdt#720).
use std::sync::Arc;
use yrs::encoding::read::{Error, Read};
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::{Any, ClientID, ID, Update};

struct CheckedDecoder<'a>(DecoderV1<'a>);

impl<'a> Read for CheckedDecoder<'a> {
    fn read_exact(&mut self, len: usize) -> Result<&[u8], Error> {
        self.0.read_exact(len)
    }

    fn read_u8(&mut self) -> Result<u8, Error> {
        self.0.read_u8()
    }
}

impl Decoder for CheckedDecoder<'_> {
    fn reset_ds_cur_val(&mut self) {
        self.0.reset_ds_cur_val()
    }

    fn read_ds_clock(&mut self) -> Result<u32, Error> {
        self.0.read_ds_clock()
    }

    fn read_ds_len(&mut self) -> Result<u32, Error> {
        self.0.read_ds_len()
    }

    fn read_left_id(&mut self) -> Result<ID, Error> {
        self.0.read_left_id()
    }

    fn read_right_id(&mut self) -> Result<ID, Error> {
        self.0.read_right_id()
    }

    fn read_client(&mut self) -> Result<ClientID, Error> {
        self.0.read_client()
    }

    fn read_info(&mut self) -> Result<u8, Error> {
        let flags = self.0.read_info()?;
        if flags & 0x10 != 0 {
            return Err(Error::UnexpectedValue);
        }
        Ok(flags)
    }

    fn read_parent_info(&mut self) -> Result<bool, Error> {
        self.0.read_parent_info()
    }

    fn read_type_ref(&mut self) -> Result<u8, Error> {
        self.0.read_type_ref()
    }

    fn read_len(&mut self) -> Result<u32, Error> {
        self.0.read_len()
    }

    fn read_any(&mut self) -> Result<Any, Error> {
        self.0.read_any()
    }

    fn read_json(&mut self) -> Result<Any, Error> {
        self.0.read_json()
    }

    fn read_key(&mut self) -> Result<Arc<str>, Error> {
        self.0.read_key()
    }

    fn read_to_end(&mut self) -> Result<&[u8], Error> {
        self.0.read_to_end()
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<Update, Error> {
    Update::decode(&mut CheckedDecoder(DecoderV1::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_reserved_info_bit_from_upstream_pending_state_crash() {
        let corrupted = [0x01, 0x01, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(Update::decode_v1(&corrupted).is_ok());
        assert!(decode(&corrupted).is_err());
    }

    #[test]
    fn permits_canonical_empty_update() {
        assert!(decode(&[0, 0]).is_ok());
    }
}
