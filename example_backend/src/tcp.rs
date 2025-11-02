use std::{
    error::Error,
    io::{self, IoSlice, Read, Write},
    net::TcpStream,
};

use bevy_replicon::bytes::{Buf, Bytes};

pub(super) fn read_message(stream: &mut TcpStream) -> io::Result<(u8, Bytes)> {
    let mut header = [0; 3];
    match stream.peek(&mut header)? {
        0 => return Err(io::ErrorKind::UnexpectedEof.into()), // Socket was closed.
        1..3 => return Err(io::ErrorKind::WouldBlock.into()), // Wait for full header.
        3.. => (),
    }

    let channel_id = header[0];
    let message_size = u16::from_le_bytes([header[1], header[2]]);

    let mut message = vec![0; header.len() + message_size as usize];
    stream.read_exact(&mut message)?;

    let mut message = Bytes::from(message);
    message.advance(header.len());

    Ok((channel_id, message))
}

pub(super) fn send_message(
    stream: &mut TcpStream,
    channel_id: usize,
    message: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let message_size: u16 = message.len().try_into()?;
    let channel_id: &[u8] = &[channel_id.try_into()?];
    let message_size = &message_size.to_le_bytes();
    let packet = [
        IoSlice::new(channel_id),
        IoSlice::new(message_size),
        IoSlice::new(message),
    ];

    // Write as a single message to avoid splitting between packets.
    let len = stream.write_vectored(&packet)?;
    if len != packet.iter().map(|s| s.len()).sum::<usize>() {
        return Err(Box::new(io::Error::from(io::ErrorKind::UnexpectedEof)));
    }

    Ok(())
}
