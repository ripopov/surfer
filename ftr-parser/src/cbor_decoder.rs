use crate::types::FtrResult;
use std::io::{Read, Seek, SeekFrom};

const ONE_BYTE: u8 = 24;
const TWO_BYTES: u8 = 25;
const FOUR_BYTES: u8 = 26;
const EIGHT_BYTES: u8 = 27;
const _BREAK: u8 = 31;

const TYPE_UNSIGNED_INT: u8 = 0x00;
const TYPE_NEGATIVE_INT: u8 = 0x01;
const TYPE_BYTE_STRING: u8 = 0x02;
const TYPE_TEXT_STRING: u8 = 0x03;
const TYPE_ARRAY: u8 = 0x04;
const TYPE_MAP: u8 = 0x5;
const TYPE_TAG: u8 = 0x06;
const TYPE_FLOAT_SIMPLE: u8 = 0x07;

#[allow(dead_code)]
const FALSE: u8 = 0x14;
const TRUE: u8 = 0x15;
const SINGLE_PRECISION_FLOAT: u8 = 0x1a;
const DOUBLE_PRECISION_FLOAT: u8 = 0x1b;

pub struct CborDecoder<R> {
    pub(crate) input_stream: R,
    peek_buf: Vec<u8>,
}

impl<R: Read + Seek> CborDecoder<R> {
    pub fn new(input_stream: R) -> Self {
        let mut peek_buf = vec![0u8; 1];
        peek_buf.clear();
        Self {
            input_stream,
            peek_buf,
        }
    }

    pub fn read_tag(&mut self) -> FtrResult<i64> {
        let length = self.read_major_type(TYPE_TAG)?;
        self.read_unsigned_int(length, false)
    }

    pub fn read_major_type(&mut self, major_type: u8) -> FtrResult<u8> {
        let mut buf = vec![0u8; 1];
        if !self.peek_buf.is_empty() {
            buf[0] = self.peek_buf[0];
            self.peek_buf.clear();
        } else {
            self.input_stream
                .read_exact(&mut buf)
                .map_err(|e| e.to_string())?
        };

        if major_type != ((buf[0] >> 5) & 0x07) {
            Err("Incorrect major type!".into())
        } else {
            Ok(buf[0] & 0x1F)
        }
    }

    pub fn read_major_type_with_size(&mut self, major_type: u8) -> FtrResult<i64> {
        let length = self.read_major_type(major_type)?;
        self.read_unsigned_int(length, true)
    }

    pub fn read_major_type_exact(&mut self, major_type: u8, expected: u8) -> FtrResult<()> {
        let actual = self.read_major_type(major_type)?;
        if actual != expected {
            Err(format!(
                "Expected ({}) and received ({}) subtype do not match!",
                expected, actual
            ))
        } else {
            Ok(())
        }
    }

    pub fn read_array_length(&mut self) -> FtrResult<i64> {
        self.read_major_type_with_size(TYPE_ARRAY)
    }

    pub fn read_unsigned_int(&mut self, length: u8, _break_allowed: bool) -> FtrResult<i64> {
        if length < 24 {
            Ok(length as i64)
        } else if length == ONE_BYTE {
            self.read_unsigned_int_8()
        } else if length == TWO_BYTES {
            self.read_unsigned_int_16()
        } else if length == FOUR_BYTES {
            self.read_unsigned_int_32()
        } else if length == EIGHT_BYTES {
            self.read_unsigned_int_64()
        } else {
            Ok(-1)
        }
    }

    fn read_unsigned_int_8(&mut self) -> FtrResult<i64> {
        let mut buf = vec![0u8; 1];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok(buf[0] as i64)
    }

    fn read_unsigned_int_16(&mut self) -> FtrResult<i64> {
        let mut buf = vec![0u8; 2];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok((buf[0] as i64) << 8 | (buf[1] as i64))
    }

    fn read_unsigned_int_32(&mut self) -> FtrResult<i64> {
        let mut buf = vec![0u8; 4];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok((buf[0] as i64) << 24 | (buf[1] as i64) << 16 | (buf[2] as i64) << 8 | (buf[3] as i64))
    }

    fn read_unsigned_int_64(&mut self) -> FtrResult<i64> {
        let mut buf = vec![0u8; 8];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok((buf[0] as i64) << 56
            | (buf[1] as i64) << 48
            | (buf[2] as i64) << 40
            | (buf[3] as i64) << 32
            | (buf[4] as i64) << 24
            | (buf[5] as i64) << 16
            | (buf[6] as i64) << 8
            | (buf[7] as i64))
    }

    pub fn read_boolean(&mut self) -> FtrResult<bool> {
        let b = self.read_major_type(TYPE_FLOAT_SIMPLE)?;
        Ok(b == TRUE)
    }

    #[allow(dead_code)]
    pub fn read_double(&mut self) -> FtrResult<f64> {
        self.read_major_type_exact(TYPE_FLOAT_SIMPLE, DOUBLE_PRECISION_FLOAT)?;

        Ok(f64::from_be_bytes(
            self.read_unsigned_int_64()?.to_be_bytes(),
        ))
    }

    pub fn read_float(&mut self) -> FtrResult<f32> {
        self.read_major_type_exact(TYPE_FLOAT_SIMPLE, SINGLE_PRECISION_FLOAT)?;

        Ok(f32::from_be_bytes(
            (self.read_unsigned_int_32()? as u32).to_be_bytes(),
        ))
    }

    pub fn read_byte_string(&mut self) -> FtrResult<Vec<u8>> {
        let len = self.read_major_type_with_size(TYPE_BYTE_STRING)?;
        let mut buf = vec![0u8; len as usize];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    }

    pub fn skip_byte_string(&mut self) -> FtrResult<()> {
        self.skip_byte_string_len().map(|_| ())
    }

    pub fn skip_byte_string_len(&mut self) -> FtrResult<u64> {
        let len = self.read_major_type_with_size(TYPE_BYTE_STRING)?;
        self.input_stream
            .seek(SeekFrom::Current(len))
            .map_err(|e| e.to_string())?;
        u64::try_from(len).map_err(|_| "Negative byte-string length".to_string())
    }

    pub fn read_int(&mut self) -> FtrResult<i64> {
        let mut buf = vec![0u8; 1];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;

        let ui = self.expect_integer_type(buf[0])?;

        Ok(ui ^ self.read_unsigned_int(buf[0] & 0x1f, false)?)
    }

    pub fn expect_integer_type(&mut self, ib: u8) -> FtrResult<i64> {
        let major_type = ib >> 5;
        if (major_type != TYPE_UNSIGNED_INT) && (major_type != TYPE_NEGATIVE_INT) {
            Err("Expected Integer type!".into())
        } else {
            Ok(-(major_type as i64))
        }
    }

    pub fn read_map_length(&mut self) -> FtrResult<i64> {
        self.read_major_type_with_size(TYPE_MAP)
    }

    pub fn read_text_string(&mut self) -> FtrResult<String> {
        let len = Self::read_major_type_with_size(self, TYPE_TEXT_STRING)?;
        if len < 0 {
            return Err("Infinite length text is not supported!".into());
        }
        let mut buf = vec![0u8; len as usize];
        self.input_stream
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        match String::from_utf8(buf) {
            Ok(string) => Ok(string),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn peek(&mut self) -> FtrResult<i64> {
        self.peek_buf = vec![0u8; 1];
        self.input_stream
            .read_exact(&mut self.peek_buf)
            .map_err(|e| e.to_string())?;
        Ok(self.peek_buf[0] as i64)
    }
}
