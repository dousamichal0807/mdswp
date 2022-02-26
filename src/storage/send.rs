use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use crate::segment::SeqNumber;
use crate::segment::SeqSegment;
use crate::util::conn_timeout;
use crate::util::conn_write_finished;
use crate::util::WINDOW_SIZE;

pub(crate) struct SendStorage {
    segment_queue_sent: VecDeque<(SeqNumber, Instant)>,
    segment_registry: BTreeMap<SeqNumber, SeqSegment>,
    highest_sent: SeqNumber,
    next_seq_num: SeqNumber,
    window: SeqNumber
}

impl SendStorage {
    pub(crate)fn new(window: SeqNumber) -> Self {
        Self {
            segment_queue_sent: VecDeque::new(),
            segment_registry: BTreeMap::new(),
            highest_sent: window.wrapping_sub(1),
            next_seq_num: window,
            window
        }
    }

    pub(crate) fn pop(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        self.__try_pop_sent()
            .or_else(|| self.__try_pop_unsent())
    }

    pub(crate) fn push(&mut self, segment: SeqSegment) -> io::Result<SeqNumber> {
        match segment {
            // If we should push finish or data check that we have not finished yet:
            SeqSegment::Finish |
            SeqSegment::Data { .. } => match self.finished() {
                true => Result::Err(conn_write_finished()),
                false => Result::Ok(self.__push_unchecked(segment))
            }
            // If we should push accept check that we have not pushed any segment
            // yet:
            SeqSegment::Accept => match self.segment_registry.is_empty() {
                true => Result::Ok(self.__push_unchecked(segment)),
                false => unreachable!("Cannot push Accept if it is not the first \
                    segment to pe pushed")
            }
        }
    }

    pub(crate) fn finished(&self) -> bool {
        if self.segment_registry.is_empty() {
            return false;
        }
        let last_seq_num = self.next_seq_num.wrapping_sub(1);
        match self.segment_registry.get(&last_seq_num).unwrap() {
            SeqSegment::Finish => true,
            _other => false
        }
    }

    fn __try_pop_sent(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        let (peek_seq_num, peek_last_pop) = self.segment_queue_sent.pop_front()?;
        let elapsed = peek_last_pop.elapsed();
        // If the segment should be sent, e.g. if 3/4 of connection timeout elapsed
        // from last send, get the segment return it as a value and push the segment
        // to the back:
        if elapsed > (3 * conn_timeout() / 4) {
            let segment = self.segment_registry.get(&peek_seq_num).unwrap().clone();
            self.segment_queue_sent.push_back((peek_seq_num, Instant::now()));
            Option::Some((peek_seq_num, segment))
        }
        // If the time did not elapse, push the segment to its original position
        // (the front) and return Option::None:
        else {
            self.segment_queue_sent.push_front((peek_seq_num, peek_last_pop));
            Option::None
        }
    }

    fn __try_pop_unsent(&mut self) -> Option<(SeqNumber, SeqSegment)> {
        let offset = self.highest_sent.wrapping_sub(self.window);
        match offset.cmp(&WINDOW_SIZE) {
            // This cannot be reached:
            Ordering::Greater => unreachable!("Some sent segments are outside of the protocol window"),
            // If equal, then we have just exceeded the protocol window:
            Ordering::Equal => Option::None,
            // If less, then we are still inside the window:
            Ordering::Less => {
                let segment = self.segment_registry.get(&self.highest_sent)?.clone();
                let segment_seq_num = self.highest_sent;
                let new_highest = self.highest_sent.wrapping_add(1);
                self.segment_queue_sent.push_back((self.highest_sent, Instant::now()));
                self.highest_sent = new_highest;
                Option::Some((segment_seq_num, segment))
            }
        }
    }

    fn __push_unchecked(&mut self, segment: SeqSegment) -> SeqNumber {
        // Does not check if pushing given segment to the queue is valid
        let curr_seq_num = self.next_seq_num;
        let next_seq_num = curr_seq_num.wrapping_add(1);
        self.segment_registry.insert(curr_seq_num, segment);
        self.next_seq_num = next_seq_num;
        curr_seq_num
    }
}