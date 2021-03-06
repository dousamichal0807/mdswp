use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::num::Wrapping;
use std::time::Instant;

use crate::segment::SequenceNumber;
use crate::segment::SequentialSegment;
use crate::util::conn_invalid_seq_num;
use crate::util::conn_timeout;
use crate::util::conn_unreliable;
use crate::util::WINDOW_SIZE;

pub(crate) struct RecvStorage {
    segments: BTreeMap<SequenceNumber, SequentialSegment>,
    pop_queue: VecDeque<SequentialSegment>,
    window: SequenceNumber,
    lower_bound: SequenceNumber,
    highest: Option<SequenceNumber>,
    last_ack: Option<(SequenceNumber, Instant)>,
    urgent_ack: bool
}

impl RecvStorage {
    pub(crate) fn new_by_peer(establish_seq_num: SequenceNumber) -> Self {
        let window = establish_seq_num + Wrapping(1);
        Self {
            segments: BTreeMap::new(),
            pop_queue: VecDeque::new(),
            window,
            lower_bound: window,
            highest: Option::Some(establish_seq_num),
            last_ack: Option::None,
            urgent_ack: false,
        }
    }

    /// Creates a new instance. This constructor is used when connection was *not*
    /// established by peer.
    ///
    /// This contructor pushes [`Accept`] segment automatically. No need for manual
    /// acknowledge of the [`Accept`] segment.
    ///
    /// [`Accept`]: SeqSegment::Accept
    pub(crate) fn new_by_local(accept_seq_num: SequenceNumber) -> Self {
        let window = accept_seq_num + Wrapping(1);
        let mut segments = BTreeMap::new();
        segments.insert(accept_seq_num, SequentialSegment::Accept);
        Self {
            segments,
            pop_queue: VecDeque::new(),
            window,
            lower_bound: accept_seq_num,
            highest: Option::Some(accept_seq_num),
            last_ack: Option::None,
            urgent_ack: true,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.segments.clear();
    }

    pub(crate) fn recv(&mut self, seq_num: SequenceNumber, segment: SequentialSegment) -> io::Result<()> {
        // If we are out of the possible range, return error immediately:
        self.__check_valid_seq_num(seq_num)?;
        // Check presence of the segment. If it is present set urgent_ack
        // accordingly and there is nothing to do more:
        if self.__check_presence_of(seq_num, &segment)? {
            self.urgent_ack = true;
            return Result::Ok(());
        }
        // Otherwise continue in execution. Push the segment to the registry:
        self.segments.insert(seq_num, segment);
        // Update highest sequence number
        match self.highest {
            Option::None => self.highest = Option::Some(seq_num),
            Option::Some(old_high) => {
                // Set seq_num to self.highest if it is larger than current value of
                // self.highest
                if seq_num - old_high < WINDOW_SIZE {
                    self.highest = Option::Some(seq_num);
                }
            }
        }
        // Slide window as far as possible if we have received segment just where
        // the window starts
        if seq_num == self.window {
            self.__try_slide_window();
        }

        // Everything done
        Result::Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<SequentialSegment> {
        self.pop_queue.pop_front()
    }

    pub(crate) fn pop_acknowledge(&mut self) -> Option<SequenceNumber> {
        // Window sits on first unreceived segment, we do not want to acknowledge
        // that one:
        let curr_ack_num = self.window - Wrapping(1);
        // Based on if we have already acknowledged:
        match self.last_ack {
            // If we have not acknowledged yet and still didn't receive single
            // segment, nothing to do. If we have received something, acknowledge
            // that.
            Option::None => {
                self.last_ack = Option::Some((curr_ack_num, Instant::now()));
                match self.window == self.lower_bound {
                    true => Option::None,
                    false => Option::Some(curr_ack_num),
                }
            },
            // If we have already acknowledged:
            Option::Some((last_ack_num, last_time)) => {
                // Acknowledge only when it is really needed:
                let elapsed = last_time.elapsed();
                let seq_num_diff = curr_ack_num - last_ack_num;
                if self.urgent_ack
                    || elapsed > conn_timeout() / 2
                    || seq_num_diff > WINDOW_SIZE >> 1
                {
                    self.urgent_ack = false;
                    let new_val = Option::Some((curr_ack_num, Instant::now()));
                    self.last_ack = new_val;
                    Option::Some(curr_ack_num)
                } else {
                    Option::None
                }
            }
        }
    }

    fn __check_valid_seq_num(&self, seq_num: SequenceNumber) -> io::Result<()> {
        // Upper bound is exclusive
        let upper_bound = self.window + WINDOW_SIZE;
        // Maximum distance from lower bound is also exclusive then:
        let max_offset = upper_bound - self.lower_bound;
        // Distance of given sequence number from lower bound is inclusive:
        let offset = seq_num - self.lower_bound;
        // That means, in condition below, there cannot be 'less or equal' operator.
        match offset < max_offset {
            true => Result::Ok(()),
            false => Result::Err(conn_invalid_seq_num(seq_num, self.lower_bound, upper_bound))
        }
    }

    fn __check_presence_of(&self, seq_num: SequenceNumber, segment: &SequentialSegment) -> io::Result<bool> {
        // Is given sequence number present?
        match self.segments.get(&seq_num) {
            // If yes, check the contents:
            Option::Some(expected) => return
                // If they are same, return that it is already present
                if segment == expected { Result::Ok(true) }
                // If they are different, that's an error:
                else { Result::Err(conn_unreliable(segment, expected)) },
            // If no, return it is not present
            Option::None => Result::Ok(false)
        }
    }

    fn __try_slide_window(&mut self) {
        // Start with current window
        let mut new_window = self.window;
        loop {
            // Try to get a segment where new_window is:
            match self.segments.get(&new_window) {
                // If there is some value, we can slide by one more
                Option::Some(..) => new_window = new_window + Wrapping(1),
                // If there is no value, prevent window from sliding further
                Option::None => break
            }
        }
        // Move to queue segments that were in the window, but now the window "left
        // them behind":
        let mut index = self.window;
        while index != new_window {
            self.pop_queue.push_back(self.segments[&index].clone());
            index += Wrapping(1);
        }
        // Assign a new window
        self.window = new_window;
        // New minimum possible value of lower bound
        let min_lower_bound = new_window - WINDOW_SIZE;
        // Is self.lower_bound < min_lower_bound? Then lower bound should be updated
        // to min_lower_bound. Note how overflow works here:
        if min_lower_bound - self.lower_bound < WINDOW_SIZE {
            // Delete all old segments:
            let mut index = self.lower_bound;
            while index != min_lower_bound {
                self.segments.remove(&index).unwrap();
                index += Wrapping(1);
            }
            // Set lower bound:
            self.lower_bound = min_lower_bound;
        };

    }
}