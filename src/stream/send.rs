use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use crate::segment::SeqNumber;
use crate::segment::SeqSegment;
use crate::util::conn_timeout;
use crate::util::conn_unexp_seg;
use crate::util::conn_write_finished;
use crate::util::WINDOW_SIZE;

pub(crate) struct SendStorage {
    queue_sent: VecDeque<(SeqNumber, SeqSegment, Instant)>,
    queue_unsent: VecDeque<SeqSegment>,
    //highest_sent: Option<SeqNumber>,
    next_unsent: SeqNumber
}

impl SendStorage {
    /// Creates a new instance. This method is used when the Establish segment has
    /// been sent by the peer.
    ///
    /// Segment will be sent automatically as soon as possible.
    pub(crate) fn new_by_peer(accept_seq_num: SeqNumber) -> Self {
        Self {
            queue_sent: VecDeque::new(),
            queue_unsent: vec![SeqSegment::Accept].into(),
            next_unsent: accept_seq_num
        }
    }

    pub(crate) fn new_by_local(establish_seq_num: SeqNumber) -> Self {
        Self {
            queue_sent: VecDeque::new(),
            queue_unsent: VecDeque::new(),
            next_unsent: establish_seq_num.wrapping_add(1),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.queue_unsent.clear();
        self.queue_sent.clear();
    }

    pub(crate) fn pop(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        self.__try_pop_sent().or_else(|| self.__try_pop_unsent())
    }

    pub(crate) fn push(&mut self, segment: SeqSegment) -> io::Result<()> {
        match segment {
            // If we should push finish or data check that we have not finished yet:
            SeqSegment::Finish |
            SeqSegment::Data { .. } => if self.__finished_unchecked() {
                Result::Err(conn_write_finished())
            } else {
                self.queue_unsent.push_back(segment);
                Result::Ok(())
            }
            // If we should push accept check that we have not pushed any segment
            // yet:
            SeqSegment::Accept => {
                assert!(self.is_empty(), "Cannot push ACCEPT if it is not the \
                    first segment to pe pushed");
                self.queue_unsent.push_back(segment);
                Result::Ok(())
            }
        }
    }

    pub fn acknowledge(&mut self, ack_num: SeqNumber) -> io::Result<()> {
        // Acknowledgement segment must be in bounds
        let offset = self.next_unsent.wrapping_sub(ack_num);
        if offset >= WINDOW_SIZE {
            return Result::Err(conn_unexp_seg(
                format!("Invalid acknowledgement number: {}", ack_num)))
        }
        // Remove old segments from the queue of sent segments
        for index in (0..self.queue_sent.len()).into_iter().rev() {
            let (seq_num, _, _) = self.queue_sent[index];
            // if ack_num > seq_num, drop the segment
            if ack_num.wrapping_sub(seq_num) < WINDOW_SIZE {
                self.queue_sent.remove(index);
            }
        }
        Result::Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queue_sent.is_empty() && self.queue_unsent.is_empty()
    }

    fn __try_pop_sent(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        let (peek_seq_num, segment, peek_last_pop) = self.queue_sent.pop_front()?;
        let elapsed = peek_last_pop.elapsed();
        // If the segment should be sent, e.g. if 3/4 of connection timeout elapsed
        // from last send, get the segment return it as a value and push the segment
        // to the back:
        if elapsed > (conn_timeout() / 2) {
            self.queue_sent.push_back((peek_seq_num, segment.clone(), Instant::now()));
            println!("send seq_num={}, segment={:?}", peek_seq_num, &segment);
            Option::Some((peek_seq_num, segment))
        }
        // If the time did not elapse, push the segment to its original position
        // (the front) and return Option::None:
        else {
            self.queue_sent.push_front((peek_seq_num, segment, peek_last_pop));
            Option::None
        }
    }

    fn __try_pop_unsent(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        // Pop only if there are less than WINDOW_SIZE waiting for
        // acknowledge:
        assert!(self.queue_sent.len() <= WINDOW_SIZE as usize);
        if self.queue_sent.len() == WINDOW_SIZE as usize {
            return Option::None;
        }
        // Seqence number of the segment that might be popped:
        let pop_unsent = self.next_unsent;
        // Try popping the segment
        let pop_unsent_seg = match self.queue_unsent.pop_front() {
            Option::Some(s) => s,
            Option::None => return Option::None,
        };
        // If successful, increase next_unsent by one and add segment to sent
        // segments
        self.next_unsent = pop_unsent.wrapping_add(1);
        self.queue_sent.push_back((pop_unsent, pop_unsent_seg.clone(), Instant::now()));
        // Return value
        println!("send seq_num={}, segment={:?}", pop_unsent, &pop_unsent_seg);
        Option::Some((pop_unsent, pop_unsent_seg))
    }

    fn __finished_unchecked(&self) -> bool {
        // This method is used with caution. This method assumes that situation
        // when all data INCLUDING FINISH were sent and acknowledged cannot
        // happen
        if self.is_empty() { false }
        // If last segment is FINISH, write to SendStorage is finished
        else if !self.queue_unsent.is_empty() {
            *self.queue_unsent.back().unwrap() == SeqSegment::Finish
        }
        // If there is no segment in unsent check sent segments
        else {
            let highest = self.next_unsent.wrapping_sub(1);
            let (_, highest_seg, _) = self.queue_sent.iter()
                .filter(|(seq_num, _, _)| *seq_num == highest)
                .next().unwrap();
            *highest_seg == SeqSegment::Finish
        }
    }

    fn __debug(&self) {
        let mut position = Option::None;
        for index in 0..self.queue_unsent.len() {
            let segment = &self.queue_unsent[index];
            if *segment == SeqSegment::Accept {
                position = Option::Some(index);
            }
        }
        println!("Position of ACCEPT: {:?}", position);
    }
}