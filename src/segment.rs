//! This is an internal module for segment represntation. Here you can find how
//! exactly MDSWP segments work.
//!
//! For general information see [crate documentation](crate).
//!
//! # Segments
//!
//! ```text
//!                            +----------------+
//!                            |  ALL SEGMENTS  |
//!                            +----------------+
//!                                     |
//!                 +-------------------+-------------------+
//!                 |                   |                   |
//!        +-----------------+ +-----------------+ +-----------------+
//!        |   Sequential    | |   Acknowledge   | | Non-sequential  |
//!        +-----------------+ +-----------------+ +-----------------+
//!                 |                                       |
//!      +----------+----------+                     +------+------+
//!      |          |          |                     |             |
//! +--------+ +--------+ +--------+           +-----------+ +-----------+
//! | Accept | | Finish | |  Data  |           | Establish | |   Reset   |
//! +--------+ +--------+ +--------+           +-----------+ +-----------+
//! ```
//!
//! Segment can be of several types. Segments are divided into two types: sequential
//! and non-sequential. Successful transmission of sequential segment must be
//! confirmed by the [`Acknowledge`] segment, whereas non-sequential are not
//! *acknowledged*. [`Acknowledge`] segment is not acknowledged again since that
//! would result in an *acknowledge storm*, but in the diagram above it is separated
//! from non-sequential segments for its special meaning.
//!
//! To distinguish between segment types in the UDP datagram, each segment type is
//! given a unique instruction code. Instruction code occupies the first byte in the
//! segment. Put another way, segment type is determined by the first byte of the
//! segment, where the instruction code is located.
//!
//! # Opening a connection
//!
//! Connection is established using [`Establish`] segment. The peer should respond
//! with [`Accept`]. Since [`Accept`] is a sequential segment, [`Acknowledge`]
//! segment should be sent as response. If peer wants do decline a connection
//! [`Reset`] should be sent
//!
//! # Closing a connection
//!
//! TODO: FINISH segment, RESET segment
//!
//! # Sending data
//!
//! Data are sent using [`Data`] segment.
//!
//! TODO: DATA segment
//!
//! [`Accept`]:      SequentialSegment::Accept
//! [`Acknowledge`]: Segment::Acknowledge
//! [`Data`]:        SequentialSegment::Data
//! [`Establish`]:   Segment::Establish
//! [`Finish`]:      SequentialSegment::Finish
//! [`Reset`]:       Segment::Reset

use std::borrow::Borrow;
use std::cmp::min;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::mem::size_of;
use std::num::Wrapping;
use std::ops::Index;
use std::ops::Range;
use std::ops::RangeFrom;
use std::slice::SliceIndex;

/// Type used as instruction code
pub(crate) type InstrCode = u8;

/// Type used as sequence number.
pub(crate) type SequenceNumber = Wrapping<u8>;

/// Maximum length of the segment as [`usize`].
pub(crate) const MAX_SEGMENT_LEN: usize = 512;

/// Maximum length of carried data in [`DataSegment`].
pub(crate) const MAX_DATA_LEN: usize = MAX_SEGMENT_LEN - (size_of::<InstrCode>() + size_of::<SequenceNumber>());

const RANGE_INSTR_CODE: Range<usize> = 0..size_of::<InstrCode>();
const RANGE_SEQ_NUM: Range<usize> = RANGE_INSTR_CODE.end..(RANGE_INSTR_CODE.end + size_of::<SequenceNumber>());
const RANGE_DATA: RangeFrom<usize> = RANGE_SEQ_NUM.end..;
const INSTR_CODE_RESET:       InstrCode = 0x00;
const INSTR_CODE_ESTABLISH:   InstrCode = 0x01;
const INSTR_CODE_ACKNOWLEDGE: InstrCode = 0x10;
const INSTR_CODE_ACCEPT:      InstrCode = 0x11;
const INSTR_CODE_FINISH:      InstrCode = 0x12;
const INSTR_CODE_DATA:        InstrCode = 0x13;

/// Returns the instruction code for given segment (byte sequence).
fn instr_code_of(segment: &[u8]) -> Option<InstrCode> {
    let bytes = segment.get(RANGE_INSTR_CODE)?;
    Option::Some(InstrCode::from_be_bytes(bytes.try_into().unwrap()))
}

/// Returns the seqence number for given segment (byte sequence).
fn seq_num_of(segment: &[u8]) -> Option<SequenceNumber> {
    let bytes = segment.get(RANGE_SEQ_NUM)?;
    Option::Some(Wrapping(u8::from_be_bytes(bytes.try_into().unwrap())))
}

/// Returns user data carried in given segment (byte sequence).
fn data_of(segment: &[u8]) -> Option<DataSegment> {
    let bytes = segment.get(RANGE_DATA)?;
    bytes.try_into().ok()
}


///
/// [`Acknowledge`]: Segment::Acknowledge
/// [`instr_code`]: Segment::instr_code
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum Segment {
    /// [`Establish`] message is used to establish a connection. This message must be the first sent
    /// message. The receiver should reply with [`Accept`], if the receiver accepts a connection.
    /// For denying the connection, receiver can send [`Reset`], but it is not mandatory.
    ///
    /// [`Accept`]: SequentialSegment::Accept
    /// [`Establish`]: Segment::Establish
    /// [`Reset`]: Segment::Reset
    Establish {
        /// Starting sequence number. Next sequential segment must have sequence number equal to
        /// `start_seq_num + 1`.
        start_seq_num: SequenceNumber
    },

    /// [`Reset`] message is used to immediately stop communicating. After receiving [`Reset`], no
    /// more data should be sent. It is also used to reject incoming connection request. See
    /// [`Establish`] for more details.
    ///
    /// [`Establish`]: Self::Establish
    /// [`Reset`]: Self::Reset
    Reset,

    /// [`Acknowledge`] is sent to confirm successful receive of a [`Sequential`] segment.
    ///
    /// [`Acknowledge`]: Segment::Acknowledge
    /// [`Sequential`]: Segment::Sequential
    Acknowledge {
        /// Sequence number of segment which should be acknowledged.
        seq_num: SequenceNumber
    },

    /// This variant is for segments that should be acknowledged. Contains sequential number
    /// and the variant of the [`SequentialSegment`].
    Sequential {
        /// Sequence number of the segment.
        seq_num: SequenceNumber,
        /// The actual message carried inside the segment. For variants see [`SequentialSegment`].
        variant: SequentialSegment,
    },
}

impl Segment {
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Establish { start_seq_num } =>
                INSTR_CODE_ESTABLISH.to_be_bytes().iter()
                    .chain(start_seq_num.0.to_be_bytes().iter())
                    .map(|&b| b)
                    .collect(),
            Self::Reset => INSTR_CODE_RESET.to_be_bytes().to_vec(),
            Self::Acknowledge { seq_num } =>
                INSTR_CODE_ACKNOWLEDGE.to_be_bytes().iter()
                    .chain(seq_num.0.to_be_bytes().iter())
                    .map(|&b| b)
                    .collect(),
            Self::Sequential { seq_num, variant } => match variant {
                SequentialSegment::Accept =>
                    INSTR_CODE_ACCEPT.to_be_bytes().to_vec().into_iter()
                        .chain(seq_num.0.to_be_bytes())
                        .collect(),
                SequentialSegment::Finish =>
                    INSTR_CODE_FINISH.to_be_bytes().to_vec().into_iter()
                        .chain(seq_num.0.to_be_bytes())
                        .collect(),
                SequentialSegment::Data { data } =>
                    INSTR_CODE_DATA.to_be_bytes().to_vec().into_iter()
                        .chain(seq_num.0.to_be_bytes())
                        .chain(data.into_iter())
                        .collect()
            }
        }
    }
}

impl TryFrom<&[u8]> for Segment {
    type Error = io::Error;

    fn try_from(segment: &[u8]) -> io::Result<Self> {
        if segment.len() > MAX_SEGMENT_LEN {
            return Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid segment length",
            ));
        }

        let instr_code = instr_code_of(segment);
        let seq_num = seq_num_of(segment);
        let data = data_of(segment);

        let instr_code = match instr_code {
            Option::None => return Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid segment length"
            )),
            Option::Some(instr_code) => instr_code
        };

        match instr_code {
            // Establish segment
            INSTR_CODE_ESTABLISH => if seq_num.is_some() && data.is_none() {
                Result::Ok(Self::Establish {
                    start_seq_num: seq_num.unwrap()
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid ESTABLISH segment",
                ))
            },
            // Reset segment
            INSTR_CODE_RESET => if seq_num.is_none() {
                Result::Ok(Self::Reset)
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid RESET segment",
                ))
            }
            // Accept segment
            INSTR_CODE_ACCEPT => if seq_num.is_some() && data.is_none() {
                Result::Ok(Self::Sequential {
                    seq_num: seq_num.unwrap(),
                    variant: SequentialSegment::Accept,
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid ACCEPT segment",
                ))
            },
            // Finish segment
            INSTR_CODE_FINISH => if seq_num.is_some() && data.is_none() {
                Result::Ok(Self::Sequential {
                    seq_num: seq_num.unwrap(),
                    variant: SequentialSegment::Finish,
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid FINISH segment",
                ))
            },
            // Acknowledge segment
            INSTR_CODE_ACKNOWLEDGE => if seq_num.is_some() && data.is_none() {
                Result::Ok(Self::Acknowledge {
                    seq_num: seq_num.unwrap()
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid ACKNOWLEDGE segment",
                ))
            },
            // Data segment
            INSTR_CODE_DATA => if seq_num.is_some() && data.is_some() {
                Result::Ok(Self::Sequential {
                    seq_num: seq_num.unwrap(),
                    variant: SequentialSegment::Data { data: data.unwrap() },
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid DATA segment",
                ))
            },
            // Other value
            _ => Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Malformed segment: unknown type",
            ))
        }
    }
}

impl Debug for Segment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Establish { start_seq_num } =>
                write!(f, "ESTABLISH (start_seq_num: {})", start_seq_num),
            Self::Reset => write!(f, "RESET"),
            Self::Acknowledge { seq_num } => write!(f, "ACKNOWLEDGE (seq_num: {})", seq_num),
            Self::Sequential { seq_num, variant } => match variant {
                SequentialSegment::Accept => write!(f, "ACCEPT (start_seq_num: {})", seq_num),
                SequentialSegment::Finish => write!(f, "FINISH (seq_num: {})", seq_num),
                SequentialSegment::Data { data } => {
                    let len = min(data.len(), 10);
                    write!(f, "DATA (seq_num: {}, data: <{} bytes> first {} bytes: {:?})",
                           seq_num, data.len(), len, &data[..len])
                }
            }
        }
    }
}

impl Display for Segment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum SequentialSegment {
    /// [`Accept`] message is used to accept incoming connection. [`Accept`] should be sent only
    /// after receiving [`Establish`]. Successful receive of [`Accept`] must be always acknowledged.
    /// After acknowledging [`Accept`], both sides can send data. See [`Acknowledge`] for more
    /// information.
    ///
    /// [`Accept`]: SequentialSegment::Accept
    /// [`Acknowledge`]: Segment::Acknowledge
    /// [`Establish`]: Segment::Establish
    Accept,

    /// [`Finish`] message is used to close the connection. Note that this only closes only sending
    /// for one side. Other side can still send some data.
    ///
    /// [`Finish`]: SequentialSegment::Finish
    Finish,

    /// Represents a segment containing the user data.
    Data {
        /// Data stored inside the segment
        data: DataSegment
    },
}

impl Debug for SequentialSegment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => write!(f, "ACCEPT"),
            Self::Data { data } => {
                let len = min(data.len(), 10);
                write!(f, "DATA ({} bytes total, first {} bytes: {:?})",
                       data.len(), len, &data[..len])
            }
            Self::Finish => write!(f, "FINISH")
        }
    }
}

/// Represents carried data in [`SequentialSegment::Data`].
///
/// # Immutability
///
/// Once [`DataSegment`] is constructed, it cannot be changed in any way.
///
/// # Construction using [`TryFrom`]`<&[u8]>`
///
/// [`DataSegment`] can be created only from slice of bytes (`&[u8]`). Slice content
/// is copied.
///
/// Note that MDSWP protocol uses UDP under the hood. That means the maximum length
/// of  is 2<sup>16</sup>&nbsp;&ndash;&nbsp;1 = 65&nbsp;535 = [`u16::MAX`]
/// bytes. Because the first byte is occupied by instruction code and the second by
/// sequence number, maximum length of the data must be at most
/// 2<sup>16</sup>&nbsp;&ndash;&nbsp;3 = 65&nbsp;533 = [`MAX_DATA_LEN`] bytes.
///
/// When the slice is longer than possible, conversion will fail with [`io::Error`].
/// If the slice is not so long, slice content gets copied and conversion will
/// succeed.
///
/// # Borrowing as a `&[u8]`
///
/// As [`DataSegment`] is constructed from byte slice which gets copied, segment can
/// also be borrowed as slice of bytes.
///
/// # Iterating over bytes
///
/// [`DataSegment`] can be converted into a iterator over [`u8`]s. This consumes the
/// [`DataSegment`] itself. It is also possible to iterate in a non-consuming manner
/// using [`iter`] method.
///
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct DataSegment {
    data: Vec<u8>,
}

impl DataSegment {
    /// Returns the length of the data segment. It is guaranteed to be at most
    /// [`MAX_DATA_LEN`] bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }
}

impl TryFrom<&[u8]> for DataSegment {
    type Error = io::Error;

    fn try_from(data: &[u8]) -> io::Result<Self> {
        if data.is_empty() {
            Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No data"
            ))
        } else if data.len() > MAX_DATA_LEN {
            Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Data too long",
            ))
        } else {
            Result::Ok(Self { data: data.to_vec() })
        }
    }
}

impl AsRef<[u8]> for DataSegment {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Borrow<[u8]> for DataSegment {
    fn borrow(&self) -> &[u8] {
        &self.data
    }
}

impl<I> Index<I> for DataSegment
    where I: SliceIndex<[u8]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &I::Output {
        &self.data[index]
    }
}

impl IntoIterator for DataSegment {
    type Item = u8;
    type IntoIter = std::vec::IntoIter<u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.into_iter()
    }
}
