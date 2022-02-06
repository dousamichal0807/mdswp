use std::borrow::Borrow;
use std::cmp::Ordering;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt;
use std::fmt::Formatter;
use std::io;
use std::num::NonZeroU16;
use std::ops;
use std::slice::SliceIndex;

pub(crate) type SequenceNumber = u8;

/// Maximum length of the segment as [`usize`].
pub(crate) const MAX_SEGMENT_LEN: usize = u16::MAX as usize;

/// Maximum length of carried data in [`DataSegment`].
pub(crate) const MAX_DATA_LEN: usize = (u16::MAX - 2) as usize;

/// Instruction code for [`Reset`](Segment::Reset) message. This constant is not exposed as public.
const INSTR_CODE_RESET: u8 = 0x00;

/// Instruction code for [`Establish`](Segment::Establish) message. This constant is not exposed as
/// public.
const INSTR_CODE_ESTABLISH: u8 = 0x01;

/// Instruction code for [`Acknowledge`](Segment::Acknowledge) message. This constant is not exposed
/// as public.
const INSTR_CODE_ACKNOWLEDGE: u8 = 0x10;

/// Instruction code for [`Accept`](SequentialSegment::Accept) message. This constant is not exposed
/// as public.
const INSTR_CODE_ACCEPT: u8 = 0x11;

/// Instruction code for [`Finish`](SequentialSegment::Finish) message. This constant is not exposed
/// as public.
const INSTR_CODE_FINISH: u8 = 0x12;

/// Instruction code for [`Data`](SequentialSegment::Data) message. This constant is not exposed as
/// public.
const INSTR_CODE_DATA: u8 = 0x13;

/// Represents a segment for the MDSWP protocol.
///
/// Segment can be of several types. For each segment type there is one variant of this enum.
///
/// # Byte representation using instruction code
///
/// Each variant is given an instruction code. Every message has different number as the
/// instruction code. This instruction code is the first byte in the segment. It means that segment
/// type is distinguished by the first byte of the segment, where the instruction code is located.
/// For getting the instruction code see [`instr_code`] method.
///
/// # Segment types
///
/// ```
///                            +----------------+
///                            |  ALL SEGMENTS  |
///                            +----------------+
///                                     |
///                 +-------------------+-------------------+
///                 |                   |                   |
///        +-----------------+ +-----------------+ +-----------------+
///        |   Sequential    | |   Acknowledge   | | Non-sequential  |
///        +-----------------+ +-----------------+ +-----------------+
///                 |                                       |
///      +----------+----------+                     +------+------+
///      |          |          |                     |             |
/// +--------+ +--------+ +--------+           +-----------+ +-----------+
/// | Accept | | Finish | |  Data  |           | Establish | |   Reset   |
/// +--------+ +--------+ +--------+           +-----------+ +-----------+
/// ```
///
/// Segments are divided into two types: sequential and non-sequential. Successful transmission of
/// sequential segment must be confirmed by the [`Acknowledge`] segment, whereas non-sequential
/// are not *acknowledged*. [`Acknowledge`] segment is not acknowledged again, but in the diagram
/// above it is separated non-sequential segments for its unique function.
///
/// [`Acknowledge`]: Segment::Acknowledge
/// [`instr_code`]: Segment::instr_code
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
    /// Returns the instruction code for the segment. See table below
    ///
    /// | Segment variant | Return value               |
    /// |-----------------|----------------------------|
    /// | [`Establish`]   | [`INSTR_CODE_ESTABLISH`]   |
    /// | [`Reset`]       | [`INSTR_CODE_RESET`]       |
    /// | [`Acknowledge`] | [`INSTR_CODE_ACKNOWLEDGE`] |
    /// | [`Accept`]      | [`INSTR_CODE_ACCEPT`]      |
    /// | [`Finish`]      | [`INSTR_CODE_FINISH`]      |
    /// | [`Data`]        | [`INSTR_CODE_DATA`]        |
    ///
    /// [`Accept`]: SequentialSegment::Accept
    /// [`Acknowledge`]: Segment::Acknowledge
    /// [`Data`]: SequentialSegment::Data
    /// [`Establish`]: Segment::Establish
    /// [`Finish`]: SequentialSegment::Finish
    /// [`Reset`]: Segment::Reset
    pub fn instr_code(&self) -> u8 {
        match self {
            Self::Reset => INSTR_CODE_RESET,
            Self::Establish { .. } => INSTR_CODE_ESTABLISH,
            Self::Acknowledge { .. } => INSTR_CODE_ACKNOWLEDGE,
            Self::Sequential { variant, .. } => match variant {
                SequentialSegment::Accept => INSTR_CODE_ACCEPT,
                SequentialSegment::Finish => INSTR_CODE_FINISH,
                SequentialSegment::Data { .. } => INSTR_CODE_DATA
            }
        }
    }

    /// Returns sequence number wrapped inside [`Option::Some`] for all variants containing it. For
    /// messages, whose successful transmission is not acknowledged, it returns [`Option::None`].
    ///
    /// As [`Acknowledge`] is not acknowledged again, this method returns [`Option::None`] also for
    /// [`Acknowledge`] segment, even if it includes sequence number.
    ///
    /// [`Acknowledge`]: Self::Acknowledge
    pub fn seq_num(&self) -> Option<u8> {
        match self {
            Self::Sequential { seq_num, .. } => Option::Some(*seq_num),
            _ => Option::None
        }
    }

    /// Returns if successful transmission of given segment must be confirmed by acknowledgement.
    /// See also [`seq_num`] method.
    ///
    /// [`seq_num`]: Self::seq_num
    pub fn is_sequential(&self) -> bool {
        match self {
            Self::Sequential { .. } => true,
            _ => false
        }
    }
}

impl TryFrom<&[u8]> for Segment {
    type Error = io::Error;

    fn try_from(segment: &[u8]) -> io::Result<Self> {
        if segment.is_empty() {
            return Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Segment is empty",
            ));
        } else if segment.len() > MAX_SEGMENT_LEN {
            return Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Segment size too large",
            ));
        }

        let instr_code = segment[0];

        match instr_code {
            // Establish segment
            INSTR_CODE_ESTABLISH => if segment.len() == 2 {
                Result::Ok(Self::Establish { start_seq_num: segment[1] })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed ESTABLISH segment",
                ))
            },
            // Reset segment
            INSTR_CODE_RESET => if segment.len() == 1 {
                Result::Ok(Self::Reset)
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed RESET segment",
                ))
            }
            // Accept segment
            INSTR_CODE_ACCEPT => if segment.len() == 2 {
                Result::Ok(Self::Sequential {
                    seq_num: segment[1],
                    variant: SequentialSegment::Accept,
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed ACCEPT segment",
                ))
            },
            // Finish segment
            INSTR_CODE_FINISH => if segment.len() == 2 {
                Result::Ok(Self::Sequential {
                    seq_num: segment[1],
                    variant: SequentialSegment::Finish,
                })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed FINISH segment",
                ))
            },
            // Acknowledgement segment
            INSTR_CODE_ACKNOWLEDGE => if segment.len() == 2 {
                Result::Ok(Self::Acknowledge { seq_num: segment[1] })
            } else {
                Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed ACKNOWLEDGE segment",
                ))
            },
            // Data segment
            INSTR_CODE_DATA => match segment.len().cmp(&2) {
                Ordering::Less => Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Malformed DATA segment",
                )),
                Ordering::Equal => Result::Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Empty DATA segment",
                )),
                Ordering::Greater => Result::Ok(Self::Sequential {
                    seq_num: segment[1],
                    variant: SequentialSegment::Data {
                        data: segment[2..].try_into().unwrap()
                    },
                })
            },
            // Other value
            _ => Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Malformed segment: unknown type",
            ))
        }
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Establish { start_seq_num } =>
                write!(f, "ESTABLISH (start_seq_num: {})", start_seq_num),
            Self::Reset => write!(f, "RESET"),
            Self::Acknowledge { seq_num } => write!(f, "ACKNOWLEDGE (seq_num: {})", seq_num),
            Self::Sequential { seq_num, variant } => match variant {
                SequentialSegment::Accept => write!(f, "ACCEPT (seq_num: {})", seq_num),
                SequentialSegment::Finish => write!(f, "FINISH (seq_num: {})", seq_num),
                SequentialSegment::Data { data } =>
                    write!(f, "DATA (seq_num: {}, data: <{} bytes>)", seq_num, data.len()),
            }
        }
    }
}

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

/// Represents carried data in [`SequentialSegment::Data`].
///
/// # Construction
///
/// [`DataSegment`] can be created only from slice of bytes (`&[u8]`). This struct implements
/// [`TryFrom`]`<&[u8]>`. Note that MDSWP protocol uses UDP under the hood. That means the maximum
/// length of the segment is 2<sup>16</sup>&nbsp;&ndash;&nbsp;1 = 65&nbsp;535 = [`u16::MAX`] bytes.
/// Because the first byte is occupied by instruction code and the second by sequence number,
/// maximum length of the data must be at most 2<sup>16</sup>&nbsp;&ndash;&nbsp;3 = 65&nbsp;533 =
/// [`MAX_DATA_LEN`] bytes. When the slice is longer than possible, conversion will fail. If the
/// slice is not so long, slice content gets copied and conversion will succeed.
///
/// # Borrow as `&[u8]`
///
/// As [`DataSegment`] is constructed from byte slice which gets copied, segment can also be
/// borrowed as slice of bytes.
pub(crate) struct DataSegment {
    data: Vec<u8>,
}

impl DataSegment {
    pub fn len(&self) -> NonZeroU16 {
        NonZeroU16::new(self.data.len() as u16).unwrap()
    }
}

impl TryFrom<&[u8]> for DataSegment {
    type Error = io::Error;

    fn try_from(data: &[u8]) -> io::Result<Self> {
        if data.len() > MAX_DATA_LEN {
            Result::Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Data too long",
            ))
        } else {
            Result::Ok(Self {
                data: data.to_vec()
            })
        }
    }
}

impl<I> ops::Index<I> for DataSegment
    where I: SliceIndex<[u8]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &I::Output {
        &self.data[index]
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

impl ops::Deref for DataSegment {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.data
    }
}