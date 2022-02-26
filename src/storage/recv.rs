use std::cmp::max;
use std::collections::BTreeMap;
use std::io;
use std::time::Instant;

use crate::segment::SeqNumber;
use crate::segment::SeqSegment;
use crate::util::conn_invalid_seq_num;
use crate::util::conn_timeout;
use crate::util::conn_unexpected_segment;
use crate::util::WINDOW_SIZE;

struct RecvWindowData {
    pub(self) window: SeqNumber,
    pub(self) lowest: Option<SeqNumber>,
    pub(self) highest: Option<SeqNumber>,
    pub(self) next_pop: SeqNumber,
}

impl RecvWindowData {
    pub(crate) fn contains_recv(&self, seq_num: SeqNumber) -> bool {
        let lowest = self.lowest.unwrap_or(self.window);
        let max_offset = self.window
            .wrapping_add(WINDOW_SIZE)
            .wrapping_sub(lowest);
        let offset = seq_num.wrapping_sub(lowest);
        offset < max_offset
    }

    pub(crate) fn contains_pop(&self, seq_num: SeqNumber) -> bool {
        let max_offset = self.window
            .wrapping_add(WINDOW_SIZE)
            .wrapping_sub(self.next_pop);
        let offset = seq_num.wrapping_sub(self.next_pop);
        offset <= max_offset
    }

    pub(crate) fn slide_window(&mut self) {
        self.slide_window_to(self.window.wrapping_add(1));
    }

    pub(crate) fn slide_window_to(&mut self, new_pos: SeqNumber) {
        assert!(new_pos.wrapping_sub(self.window) <= WINDOW_SIZE);
        self.lowest = Option::Some(max(self.lowest.unwrap(), new_pos.wrapping_sub(WINDOW_SIZE)));
        self.window = new_pos;
    }

    pub(crate) fn recv(&mut self, seq_num: SeqNumber) -> io::Result<()> {
        if !self.contains_recv(seq_num) {
            Result::Err(conn_invalid_seq_num(seq_num))
        } else {
            if let Option::None = self.lowest {
                self.lowest = Option::Some(seq_num);
                self.highest = Option::Some(seq_num);
            }
            let lowest = self.lowest.unwrap();
            let highest = self.highest.unwrap();
            let recvd_offset = seq_num.wrapping_sub(lowest);
            let highest_offset = highest.wrapping_sub(lowest);
            if recvd_offset >= highest_offset { self.highest = Option::Some(seq_num); }
            Result::Ok(())
        }
    }

    pub(crate) fn pop(&mut self) -> Option<SeqNumber> {
        // Do not pop sequence number that is already inside of window:
        if self.window == self.next_pop {
            Option::None
        } else {
            let curr_pop = self.next_pop;
            self.next_pop = self.next_pop.wrapping_add(1);
            Option::Some(curr_pop)
        }
    }
}

pub(crate) struct RecvStorage {
    segments: BTreeMap<SeqNumber, SeqSegment>,
    window_data: RecvWindowData,
    last_ack: Option<(SeqNumber, Instant)>,
    urgent_ack: bool
}

impl RecvStorage {
    pub(crate) fn new_by_peer(establish_seq_num: SeqNumber) -> Self {
        let window = establish_seq_num.wrapping_sub(1);
        Self {
            segments: BTreeMap::new(),
            window_data: RecvWindowData {
                window,
                lowest: Option::Some(establish_seq_num),
                highest: Option::Some(establish_seq_num),
                next_pop: window
            },
            last_ack: Option::None,
            urgent_ack: false,
        }
    }

    /// Creates a new instance. This constructor is used when connection was *not*
    /// established by peer.
    ///
    /// First acknowledge of [`Accept`](SeqSegment::Accept) segment should be sent
    /// by caller of this method. [`Accept`](SeqSegment::Accept) will not be
    /// returned by [`pop`](Self::pop) method.
    pub(crate) fn new_by_local(accept_seq_num: SeqNumber) -> Self {
        let window = accept_seq_num.wrapping_add(1);
        let mut segments = BTreeMap::new();
        segments.insert(accept_seq_num, SeqSegment::Accept);
        Self {
            segments,
            window_data: RecvWindowData {
                window,
                highest: Option::Some(accept_seq_num),
                lowest: Option::Some(accept_seq_num),
                next_pop: window
            },
            last_ack: Option::None,
            urgent_ack: false,
        }
    }

    pub(crate) fn recv(&mut self, seq_num: SeqNumber, segment: SeqSegment) -> io::Result<()> {
        self.__check_segment(seq_num, &segment)?;
        self.window_data.recv(seq_num)?;
        // If the segment has been already received try to acknowledge immediately:
        self.urgent_ack = self.segments.insert(seq_num, segment).is_some();
        // Slide the protocol window as much as possible. Start with current window
        // position:
        let mut new_win_pos = self.window_data.window;
        // Add one for each present segment and stop when the next segment is
        // missing:
        while self.segments.get(&new_win_pos).is_some() {
            new_win_pos = new_win_pos.wrapping_add(1);
        }
        // Set the new value
        self.window_data.slide_window_to(new_win_pos);
        Result::Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<SeqSegment> {
        let curr_pop = self.window_data.pop()?;
        Option::Some(self.segments.get(&curr_pop).unwrap().clone())
    }

    pub(crate) fn pop_acknowledge(&mut self) -> Option<SeqNumber> {
        let mut curr_ack: Option<SeqNumber> = Option::None;
        // Try to calculate what sequence number should we acknowledge
        loop {
            let candidate = match curr_ack {
                Option::None => self.window_data.window,
                Option::Some(curr_ack) => curr_ack.wrapping_add(1)
            };
            match self.segments.get(&candidate) {
                Option::None => break,
                Option::Some(_) => curr_ack = Option::Some(candidate)
            }
        }
        match (self.last_ack, curr_ack) {
            // If we should not acknowledge anything and we still have not done that
            // yet, there is nothing to do
            (Option::None, Option::None) => {
                self.urgent_ack = false;
                Option::None
            },
            // If we should acknowledge something and we still have not done that yet,
            // let's do it right now:
            (Option::None, Option::Some(curr_ack)) => {
                self.last_ack = Option::Some((curr_ack, Instant::now()));
                self.urgent_ack = false;
                Option::Some(curr_ack)
            }
            // If we acknowledged everything and there is nothing to do, just save
            // that we have checked it. Evantually we send it, if urgent_ack is true:
            (Option::Some((last_ack, _)), Option::None) => {
                let result = if self.urgent_ack { Option::Some(last_ack) } else { Option::None };
                self.last_ack = Option::Some((last_ack, Instant::now()));
                self.urgent_ack = false;
                result
            },
            // If we have already acknowledged something and there is also something
            // to acknowledge:
            (Option::Some((last_ack, instant)), Option::Some(curr_ack)) => {
                let num_diff = curr_ack.wrapping_sub(last_ack);
                let elapsed = instant.elapsed();
                assert!(num_diff < WINDOW_SIZE);
                // Acknlowledge only if
                // - urgent_ack is set
                // - or the sequence number increased high,
                // - or it is long time since last acknowledge:
                if num_diff > WINDOW_SIZE / 2 || elapsed > conn_timeout() / 2 {
                    self.last_ack = Option::Some((curr_ack, Instant::now()));
                    self.urgent_ack = false;
                    Option::Some(curr_ack)
                } else {
                    Option::None
                }
            }
        }
    }

    pub(crate) fn finished(&self) -> bool {
        match self.segments.get(&self.window_data.next_pop.wrapping_sub(1)) {
            Option::None => false,
            Option::Some(segment) => *segment == SeqSegment::Finish
        }
    }

    fn __recv_seq_num(&mut self, seq_num: SeqNumber) -> io::Result<()> {
        self.window_data.recv(seq_num)
    }

    fn __check_segment(&self, seq_num: SeqNumber, segment: &SeqSegment) -> io::Result<()> {
        // If we are out of the possible range, return error immediately:
        if !self.window_data.contains_recv(seq_num) {
            return Result::Err(conn_invalid_seq_num(seq_num))
        }
        // If we have already received same segment and it is not the same it is an
        // error also:
        match self.segments.get(&seq_num) {
            Option::None => {},
            Option::Some(already_received) => if segment != already_received {
                return Result::Err(conn_unexpected_segment(format!(
                    "{:?} was expected, but different segment was received: {:?}",
                    already_received, segment
                )))
            }
        }
        // Otherwise look for what segment we have received:
        match segment {
            // If ACCEPT segment has been now received:
            SeqSegment::Accept => {
                // Accept must be the first received segment. There are two
                // possibilities:
                // 1. The segment list is still empty
                if self.segments.is_empty() {
                    Result::Ok(())
                }
                // 2. If the segment list is not empty anymore, then at least the
                // accept segment must be at least the first one in the list:
                else {
                    let lowest_seq_num = self.window_data.lowest.unwrap();
                    let is_lowest = seq_num == lowest_seq_num;
                    let lowest_is_same = *self.segments.get(&lowest_seq_num).unwrap() == SeqSegment::Accept;
                    if is_lowest && lowest_is_same { Result::Ok(()) }
                    else { Result::Err(conn_unexpected_segment(
                        "ACCEPT segment in the middle of the communication")) }
                }
            },
            // If DATA segment has been now received:
            SeqSegment::Data { .. }
            | SeqSegment::Finish => {
                // DATA segment can be the first segment sent by peer if the peer
                // established a connection, in other words, we sent ACCEPT, peer
                // then sent ACKNOWLEDGE (here are only sequential segments being
                // stored) and finally peer sent some DATA segment
                if self.segments.is_empty() {
                    return Result::Ok(())
                }
                // Note here how wrapping_sub works here. Sequence number of
                // currently received segment is higher when `diff` is larger
                // than SeqNumber::MAX - WINDOW_SIZE
                let highest = self.window_data.highest.unwrap();
                let diff = highest.wrapping_sub(seq_num);
                // Check that it is not sent after FINISH segment, if the FINISH
                // segment has been already received:
                match self.segments.get(&highest).unwrap() {
                    SeqSegment::Finish => {
                        // Has currently received DATA/FINISH segment higher
                        // sequence number than already received FINISH segment? If
                        // yes, that's an error:
                        if diff >= WINDOW_SIZE { Result::Err(conn_unexpected_segment(format!(
                            "{:?} were sent after the {:?} segment",
                            segment, SeqSegment::Finish
                        ))) }
                        // If they have same sequence number, It has been already checked if
                        // they are the same:
                        else if diff == 0 { unreachable!(
                            "This case should have been already checked") }
                        // Otherwise it is not an error:
                        else { Result::Ok(()) }
                    },
                    SeqSegment::Accept => {
                        // If segment with the higest sequence number is ACCEPT it must be alone
                        // there
                        assert_eq!(self.segments.len(), 1,
                                   "ACCEPT segment is not alone and is the most recent");
                        // And our segment must have higher sequence number:
                        if diff > SeqNumber::MAX - WINDOW_SIZE { Result::Ok(()) }
                        else { Result::Err(conn_unexpected_segment("ACCEPT segment sent later than DATA or FINISH")) }
                    }
                    SeqSegment::Data { .. } => { Result::Ok(()) }
                }
            },
            // If FINISH segment has been now received:
            /*SeqSegment::Finish => {
                // FINISH segment can be the first segment sent by peer because of
                // the same reason as DATA. In this case though the peer will only
                // listen:
                if self.segments.is_empty() {
                    return Result::Ok(())
                }
                // If packet with highest seqence number has smaller value than the
                // received finish packet it is OK.
                let highest = self.window_data.highest.unwrap();
                let offset_from_highest = seq_num.wrapping_sub();
                if offset_from_highest == 0 {
                    match self.segments.get(&seq_num).unwrap() {
                        SeqSegment::Finish => Result::Ok(()),
                        other => Result::Err(conn_unexpected_segment(format!(
                            "{:?} was expected instead of {:?}",
                            other, SeqSegment::Finish
                        )))
                    }
                }
                // If segment with the highest sequence number has the same value as
                // the received finish segment, then the last segment must be that
                // finish segment.
                else if offset_from_highest < WINDOW_SIZE { Result::Ok(()) }
                // If finish does not have the highest sequence number it is illegal
                // to send FINISH:
                // NOTE: The offset from the highest will be a large number, because
                // of how wrapping_sub works.
                else { Result::Err(conn_invalid_seq_num(seq_num)) }
            }*/
        }
    }

    fn __check_inside_window(&self, seq_num: SeqNumber) -> io::Result<()> {
        if seq_num.wrapping_sub(self.window_data.window) < WINDOW_SIZE { Result::Ok(()) }
        else { Result::Err(conn_invalid_seq_num(seq_num)) }
    }
}