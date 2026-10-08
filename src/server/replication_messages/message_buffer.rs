use alloc::collections::VecDeque;
use bytes::{Bytes, BytesMut};

/// Buffer for replicated messages so allocations can be reused.
///
/// At most [`Self::MAX_MESSAGE_BUFFER_COUNT`] updates can be cached.
/// Only messages at most [`Self::MAX_MESSAGE_BUFFER_SIZE`] will be cached.
#[derive(Debug, Default)]
pub(super) struct MessageBuffer {
    sent_message_buffer: VecDeque<Bytes>,
}

impl MessageBuffer {
    /// Max number of messages to buffer.
    const MAX_MESSAGE_BUFFER_COUNT: usize = 30;
    /// Max size of buffered messages.
    ///
    /// Small so we can store many of these. Large messages are uncommon.
    const MAX_MESSAGE_BUFFER_SIZE: usize = 3_000;

    /// Get a [`BytesMut`] for a new message.
    pub(super) fn get(&mut self, message_size: usize) -> BytesMut {
        if message_size <= Self::MAX_MESSAGE_BUFFER_SIZE {
            // Cap at 10 to mitigate worst case where the entire buffer is unavailable and there are many messages to send.
            for _ in 0..self.sent_message_buffer.len().min(10) {
                let Some(buff) = self.sent_message_buffer.pop_front() else {
                    break;
                };
                match buff.try_into_mut() {
                    Ok(mut buff) => {
                        buff.clear();
                        buff.reserve(message_size.saturating_sub(buff.capacity()));
                        return buff;
                    }
                    Err(buff) => {
                        self.sent_message_buffer.push_back(buff);
                    }
                }
            }
        }

        // Fallback to allocation
        BytesMut::with_capacity(message_size)
    }

    /// Returns the message as a copyable [`Bytes`].
    pub(super) fn cache(&mut self, message: BytesMut) -> Bytes {
        let msg: Bytes = message.into();
        if msg.len() <= Self::MAX_MESSAGE_BUFFER_SIZE
            && self.sent_message_buffer.len() < Self::MAX_MESSAGE_BUFFER_COUNT
        {
            self.sent_message_buffer.push_back(msg.clone());
        }
        msg
    }
}
