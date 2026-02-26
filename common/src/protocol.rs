use anyhow::{Result, bail};
use std::io::{ErrorKind, Read, Write};

/// Check if an error indicates a peer disconnection (EOF, broken pipe, or reset).
///
/// Shared by both client and server for consistent disconnect detection.
pub fn is_disconnect(e: &anyhow::Error) -> bool {
    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
        matches!(
            io_err.kind(),
            ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
        )
    } else {
        false
    }
}

// --- Client messages (client → server, tags 0x01-0x7F) ---

#[derive(Debug)]
pub enum ClientMsg {
    AudioSegment(Vec<i16>), // tag 0x01, payload = raw i16 LE bytes
    PauseRequest,           // tag 0x02, empty payload
    ResumeRequest,          // tag 0x03, empty payload
    InterruptTts,           // tag 0x04, empty payload
    FeedbackChoice(bool),   // tag 0x05, payload = 1 byte (0x01=continue, 0x00=retry)
    SummaryRequest,         // tag 0x06, empty payload
    CancelExchange,         // tag 0x07, empty payload
}

// --- Server messages (server → client, tags 0x80-0xFF) ---

#[derive(Debug)]
pub enum ServerMsg {
    Ready,                      // tag 0x80, empty payload
    Text(String),               // tag 0x81, payload = UTF-8
    Error(String),              // tag 0x82, payload = UTF-8
    TtsAudioChunk(Vec<i16>),    // tag 0x83, payload = raw i16 LE bytes
    TtsEnd,                     // tag 0x84, empty payload
    Feedback(String),           // tag 0x85, payload = UTF-8 (language feedback, not spoken)
    SessionSummary(String),     // tag 0x86, payload = UTF-8 markdown
    StatusNotification(String), // tag 0x87, payload = UTF-8 (e.g. "Thinking...", "Searching the web...")
}

// --- Orchestrator messages (orchestrator ↔ server, tags 0xA0-0xBF, Unix socket) ---

#[derive(Debug)]
pub enum OrchestratorMsg {
    TranscribedText(String),    // tag 0xA0, payload = UTF-8
    ResponseText(String),       // tag 0xA1, payload = UTF-8
    SessionStart(String),       // tag 0xA2, payload = UTF-8 JSON (raw string)
    SessionEnd,                 // tag 0xA3, empty payload
    FeedbackText(String),       // tag 0xA4, payload = UTF-8 (language feedback for display)
    FeedbackChoice(bool),       // tag 0xA5, payload = 1 byte (0x01=continue, 0x00=retry)
    SummaryRequest,             // tag 0xA6, empty payload
    SummaryResponse(String),    // tag 0xA7, payload = UTF-8 markdown
    StatusNotification(String), // tag 0xA8, payload = UTF-8 (e.g. "Thinking...", "Searching the web...")
    CancelExchange,             // tag 0xA9, empty payload
}

// --- Server-to-Orchestrator messages (read by orchestrator, combines server + orchestrator tags) ---

#[derive(Debug)]
pub enum ServerOrcMsg {
    Ready,                   // tag 0x80, empty payload
    Error(String),           // tag 0x82, payload = UTF-8
    TranscribedText(String), // tag 0xA0, payload = UTF-8
    FeedbackChoice(bool),    // tag 0xA5, payload = 1 byte (0x01=continue, 0x00=retry)
    SummaryRequest,          // tag 0xA6, empty payload
    CancelExchange,          // tag 0xA9, empty payload
}

// --- Wire format: [tag: u8][length: u32 LE][payload] ---

pub fn write_client_msg(w: &mut impl Write, msg: &ClientMsg) -> Result<()> {
    match msg {
        ClientMsg::AudioSegment(samples) => {
            let payload_len = samples.len() * 2; // i16 = 2 bytes
            w.write_all(&[0x01])?;
            w.write_all(&(payload_len as u32).to_le_bytes())?;
            for &s in samples {
                w.write_all(&s.to_le_bytes())?;
            }
            w.flush()?;
        }
        ClientMsg::PauseRequest => {
            w.write_all(&[0x02])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ClientMsg::ResumeRequest => {
            w.write_all(&[0x03])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ClientMsg::InterruptTts => {
            w.write_all(&[0x04])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ClientMsg::FeedbackChoice(proceed) => {
            w.write_all(&[0x05])?;
            w.write_all(&1u32.to_le_bytes())?;
            w.write_all(&[if *proceed { 0x01 } else { 0x00 }])?;
            w.flush()?;
        }
        ClientMsg::SummaryRequest => {
            w.write_all(&[0x06])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ClientMsg::CancelExchange => {
            w.write_all(&[0x07])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
    }
    Ok(())
}

pub fn read_client_msg(r: &mut impl Read) -> Result<ClientMsg> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    match tag[0] {
        0x01 => {
            if !len.is_multiple_of(2) {
                bail!("AudioSegment payload length {len} is not a multiple of 2");
            }
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            let samples: Vec<i16> = payload
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            Ok(ClientMsg::AudioSegment(samples))
        }
        0x02 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ClientMsg::PauseRequest)
        }
        0x03 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ClientMsg::ResumeRequest)
        }
        0x04 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ClientMsg::InterruptTts)
        }
        0x05 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            let proceed = payload.first().copied().unwrap_or(0x01) != 0x00;
            Ok(ClientMsg::FeedbackChoice(proceed))
        }
        0x06 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ClientMsg::SummaryRequest)
        }
        0x07 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ClientMsg::CancelExchange)
        }
        other => bail!("Unknown client message tag: 0x{other:02x}"),
    }
}

pub fn write_server_msg(w: &mut impl Write, msg: &ServerMsg) -> Result<()> {
    match msg {
        ServerMsg::Ready => {
            w.write_all(&[0x80])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ServerMsg::Text(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0x81])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        ServerMsg::Error(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0x82])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        ServerMsg::TtsAudioChunk(samples) => {
            let payload_len = samples.len() * 2; // i16 = 2 bytes
            w.write_all(&[0x83])?;
            w.write_all(&(payload_len as u32).to_le_bytes())?;
            for &s in samples {
                w.write_all(&s.to_le_bytes())?;
            }
            w.flush()?;
        }
        ServerMsg::TtsEnd => {
            w.write_all(&[0x84])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        ServerMsg::Feedback(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0x85])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        ServerMsg::SessionSummary(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0x86])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        ServerMsg::StatusNotification(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0x87])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
    }
    Ok(())
}

pub fn read_server_msg(r: &mut impl Read) -> Result<ServerMsg> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    match tag[0] {
        0x80 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ServerMsg::Ready)
        }
        0x81 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerMsg::Text(String::from_utf8(payload)?))
        }
        0x82 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerMsg::Error(String::from_utf8(payload)?))
        }
        0x83 => {
            if !len.is_multiple_of(2) {
                bail!("TtsAudioChunk payload length {len} is not a multiple of 2");
            }
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            let samples: Vec<i16> = payload
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            Ok(ServerMsg::TtsAudioChunk(samples))
        }
        0x84 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ServerMsg::TtsEnd)
        }
        0x85 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerMsg::Feedback(String::from_utf8(payload)?))
        }
        0x86 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerMsg::SessionSummary(String::from_utf8(payload)?))
        }
        0x87 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerMsg::StatusNotification(String::from_utf8(payload)?))
        }
        other => bail!("Unknown server message tag: 0x{other:02x}"),
    }
}

pub fn write_orchestrator_msg(w: &mut impl Write, msg: &OrchestratorMsg) -> Result<()> {
    match msg {
        OrchestratorMsg::TranscribedText(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0xA0])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::ResponseText(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0xA1])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::SessionStart(json) => {
            let payload = json.as_bytes();
            w.write_all(&[0xA2])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::SessionEnd => {
            w.write_all(&[0xA3])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        OrchestratorMsg::FeedbackText(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0xA4])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::FeedbackChoice(proceed) => {
            w.write_all(&[0xA5])?;
            w.write_all(&1u32.to_le_bytes())?;
            w.write_all(&[if *proceed { 0x01 } else { 0x00 }])?;
            w.flush()?;
        }
        OrchestratorMsg::SummaryRequest => {
            w.write_all(&[0xA6])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
        OrchestratorMsg::SummaryResponse(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0xA7])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::StatusNotification(text) => {
            let payload = text.as_bytes();
            w.write_all(&[0xA8])?;
            w.write_all(&(payload.len() as u32).to_le_bytes())?;
            w.write_all(payload)?;
            w.flush()?;
        }
        OrchestratorMsg::CancelExchange => {
            w.write_all(&[0xA9])?;
            w.write_all(&0u32.to_le_bytes())?;
            w.flush()?;
        }
    }
    Ok(())
}

pub fn read_orchestrator_msg(r: &mut impl Read) -> Result<OrchestratorMsg> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    match tag[0] {
        0xA0 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::TranscribedText(String::from_utf8(
                payload,
            )?))
        }
        0xA1 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::ResponseText(String::from_utf8(payload)?))
        }
        0xA2 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::SessionStart(String::from_utf8(payload)?))
        }
        0xA3 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(OrchestratorMsg::SessionEnd)
        }
        0xA4 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::FeedbackText(String::from_utf8(payload)?))
        }
        0xA5 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            let proceed = payload.first().copied().unwrap_or(0x01) != 0x00;
            Ok(OrchestratorMsg::FeedbackChoice(proceed))
        }
        0xA6 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(OrchestratorMsg::SummaryRequest)
        }
        0xA7 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::SummaryResponse(String::from_utf8(
                payload,
            )?))
        }
        0xA8 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(OrchestratorMsg::StatusNotification(String::from_utf8(
                payload,
            )?))
        }
        0xA9 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(OrchestratorMsg::CancelExchange)
        }
        other => bail!("Unknown orchestrator message tag: 0x{other:02x}"),
    }
}

/// Read a server-to-orchestrator message from the Unix socket.
///
/// Handles tags from both ServerMsg (0x80 Ready, 0x82 Error) and
/// OrchestratorMsg (0xA0 TranscribedText) since the server writes
/// both types on the same Unix socket stream.
pub fn read_server_orc_msg(r: &mut impl Read) -> Result<ServerOrcMsg> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    match tag[0] {
        0x80 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ServerOrcMsg::Ready)
        }
        0x82 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerOrcMsg::Error(String::from_utf8(payload)?))
        }
        0xA0 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            Ok(ServerOrcMsg::TranscribedText(String::from_utf8(payload)?))
        }
        0xA5 => {
            let mut payload = vec![0u8; len];
            r.read_exact(&mut payload)?;
            let proceed = payload.first().copied().unwrap_or(0x01) != 0x00;
            Ok(ServerOrcMsg::FeedbackChoice(proceed))
        }
        0xA6 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ServerOrcMsg::SummaryRequest)
        }
        0xA9 => {
            if len > 0 {
                let mut discard = vec![0u8; len];
                r.read_exact(&mut discard)?;
            }
            Ok(ServerOrcMsg::CancelExchange)
        }
        other => bail!("Unknown server-to-orchestrator message tag: 0x{other:02x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_audio_segment() {
        let samples: Vec<i16> = vec![-32768, -1, 0, 1, 32767];
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::AudioSegment(samples.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        match msg {
            ClientMsg::AudioSegment(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected AudioSegment, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_audio_segment_empty() {
        let samples: Vec<i16> = vec![];
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::AudioSegment(samples.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        match msg {
            ClientMsg::AudioSegment(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected AudioSegment, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_ready() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Ready).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerMsg::Ready));
    }

    #[test]
    fn round_trip_text() {
        let text = "Bonjour, ça va bien !".to_string();
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Text(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::Text(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_error() {
        let text = "model not found".to_string();
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Error(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::Error(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_text_empty() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Text(String::new())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::Text(decoded) => assert_eq!(decoded, ""),
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[test]
    fn multiple_messages_in_stream() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Ready).unwrap();
        write_server_msg(&mut buf, &ServerMsg::Text("hello".into())).unwrap();
        write_server_msg(&mut buf, &ServerMsg::Error("oops".into())).unwrap();

        let mut cursor = Cursor::new(buf);
        assert!(matches!(
            read_server_msg(&mut cursor).unwrap(),
            ServerMsg::Ready
        ));
        match read_server_msg(&mut cursor).unwrap() {
            ServerMsg::Text(t) => assert_eq!(t, "hello"),
            other => panic!("Expected Text, got {other:?}"),
        }
        match read_server_msg(&mut cursor).unwrap() {
            ServerMsg::Error(e) => assert_eq!(e, "oops"),
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_client_tag_errors() {
        let buf = vec![0xFF, 0, 0, 0, 0]; // unknown tag, length 0
        let mut cursor = Cursor::new(buf);
        assert!(read_client_msg(&mut cursor).is_err());
    }

    #[test]
    fn unknown_server_tag_errors() {
        let buf = vec![0xFF, 0, 0, 0, 0];
        let mut cursor = Cursor::new(buf);
        assert!(read_server_msg(&mut cursor).is_err());
    }

    // --- Task 1: PauseRequest + ResumeRequest ---

    #[test]
    fn round_trip_pause_request() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::PauseRequest).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ClientMsg::PauseRequest));
    }

    #[test]
    fn round_trip_resume_request() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::ResumeRequest).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ClientMsg::ResumeRequest));
    }

    #[test]
    fn round_trip_interrupt_tts() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::InterruptTts).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ClientMsg::InterruptTts));
    }

    // --- Task 2: TtsAudioChunk + TtsEnd ---

    #[test]
    fn round_trip_tts_audio_chunk() {
        let samples: Vec<i16> = vec![-32768, -1, 0, 1, 32767];
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::TtsAudioChunk(samples.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::TtsAudioChunk(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected TtsAudioChunk, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_tts_audio_chunk_empty() {
        let samples: Vec<i16> = vec![];
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::TtsAudioChunk(samples.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::TtsAudioChunk(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected TtsAudioChunk, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_tts_end() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::TtsEnd).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerMsg::TtsEnd));
    }

    // --- Task 3: OrchestratorMsg ---

    #[test]
    fn round_trip_transcribed_text() {
        let text = "Hello, how are you today?".to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::TranscribedText(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_response_text() {
        let text = "I'm doing well! Let's practice some English.".to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::ResponseText(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::ResponseText(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected ResponseText, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_session_start() {
        let json =
            r#"{"agent_path": "/path/to/agent.md", "session_dir": "/tmp/session"}"#.to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SessionStart(json.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::SessionStart(decoded) => assert_eq!(decoded, json),
            other => panic!("Expected SessionStart, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_session_end() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SessionEnd).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        assert!(matches!(msg, OrchestratorMsg::SessionEnd));
    }

    #[test]
    fn round_trip_transcribed_text_empty() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::TranscribedText(String::new())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(decoded) => assert_eq!(decoded, ""),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_session_start_empty() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SessionStart(String::new())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::SessionStart(decoded) => assert_eq!(decoded, ""),
            other => panic!("Expected SessionStart, got {other:?}"),
        }
    }

    #[test]
    fn unknown_orchestrator_tag_errors() {
        let buf = vec![0xFF, 0, 0, 0, 0];
        let mut cursor = Cursor::new(buf);
        assert!(read_orchestrator_msg(&mut cursor).is_err());
    }

    // --- Task 4: Multi-message stream tests ---

    #[test]
    fn multiple_client_messages_in_stream() {
        let samples: Vec<i16> = vec![100, -100];
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::AudioSegment(samples.clone())).unwrap();
        write_client_msg(&mut buf, &ClientMsg::PauseRequest).unwrap();
        write_client_msg(&mut buf, &ClientMsg::ResumeRequest).unwrap();
        write_client_msg(&mut buf, &ClientMsg::InterruptTts).unwrap();

        let mut cursor = Cursor::new(buf);
        match read_client_msg(&mut cursor).unwrap() {
            ClientMsg::AudioSegment(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected AudioSegment, got {other:?}"),
        }
        assert!(matches!(
            read_client_msg(&mut cursor).unwrap(),
            ClientMsg::PauseRequest
        ));
        assert!(matches!(
            read_client_msg(&mut cursor).unwrap(),
            ClientMsg::ResumeRequest
        ));
        assert!(matches!(
            read_client_msg(&mut cursor).unwrap(),
            ClientMsg::InterruptTts
        ));
    }

    #[test]
    fn multiple_server_messages_with_tts_in_stream() {
        let samples: Vec<i16> = vec![1000, -1000, 500];
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Ready).unwrap();
        write_server_msg(&mut buf, &ServerMsg::TtsAudioChunk(samples.clone())).unwrap();
        write_server_msg(&mut buf, &ServerMsg::TtsEnd).unwrap();

        let mut cursor = Cursor::new(buf);
        assert!(matches!(
            read_server_msg(&mut cursor).unwrap(),
            ServerMsg::Ready
        ));
        match read_server_msg(&mut cursor).unwrap() {
            ServerMsg::TtsAudioChunk(decoded) => assert_eq!(decoded, samples),
            other => panic!("Expected TtsAudioChunk, got {other:?}"),
        }
        assert!(matches!(
            read_server_msg(&mut cursor).unwrap(),
            ServerMsg::TtsEnd
        ));
    }

    #[test]
    fn multiple_orchestrator_messages_in_stream() {
        let json = r#"{"agent_path": "agent.md"}"#.to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SessionStart(json.clone())).unwrap();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::TranscribedText("hello".into()))
            .unwrap();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::ResponseText("hi there".into()))
            .unwrap();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SessionEnd).unwrap();

        let mut cursor = Cursor::new(buf);
        match read_orchestrator_msg(&mut cursor).unwrap() {
            OrchestratorMsg::SessionStart(decoded) => assert_eq!(decoded, json),
            other => panic!("Expected SessionStart, got {other:?}"),
        }
        match read_orchestrator_msg(&mut cursor).unwrap() {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "hello"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }
        match read_orchestrator_msg(&mut cursor).unwrap() {
            OrchestratorMsg::ResponseText(t) => assert_eq!(t, "hi there"),
            other => panic!("Expected ResponseText, got {other:?}"),
        }
        assert!(matches!(
            read_orchestrator_msg(&mut cursor).unwrap(),
            OrchestratorMsg::SessionEnd
        ));
    }

    // --- is_disconnect tests ---

    // --- ServerOrcMsg tests ---

    #[test]
    fn read_server_orc_msg_ready() {
        // write_server_msg produces tag 0x80 — read_server_orc_msg should parse it
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Ready).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerOrcMsg::Ready));
    }

    #[test]
    fn read_server_orc_msg_error() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Error("oops".into())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        match msg {
            ServerOrcMsg::Error(e) => assert_eq!(e, "oops"),
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn read_server_orc_msg_transcribed_text() {
        // Server writes TranscribedText via write_orchestrator_msg (tag 0xA0)
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::TranscribedText("hello".into()))
            .unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        match msg {
            ServerOrcMsg::TranscribedText(t) => assert_eq!(t, "hello"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }
    }

    #[test]
    fn read_server_orc_msg_unknown_tag() {
        let buf = vec![0xB0, 0, 0, 0, 0]; // unknown tag
        let mut cursor = Cursor::new(buf);
        assert!(read_server_orc_msg(&mut cursor).is_err());
    }

    // --- Story 6-5: Feedback message round-trip tests ---

    #[test]
    fn round_trip_client_feedback_choice_continue() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::FeedbackChoice(true)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        match msg {
            ClientMsg::FeedbackChoice(proceed) => assert!(proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_client_feedback_choice_retry() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::FeedbackChoice(false)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        match msg {
            ClientMsg::FeedbackChoice(proceed) => assert!(!proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_server_feedback() {
        let text = "RED: \"I have went\" → \"I went\" (past simple)\nBLUE: \"it is good\" → \"it's appealing\" (more natural)".to_string();
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Feedback(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::Feedback(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected Feedback, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_server_feedback_empty() {
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::Feedback(String::new())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::Feedback(decoded) => assert_eq!(decoded, ""),
            other => panic!("Expected Feedback, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_orchestrator_feedback_text() {
        let text = "RED: \"I have went\" → \"I went\" (past simple)".to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::FeedbackText(text.clone())).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::FeedbackText(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected FeedbackText, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_orchestrator_feedback_choice_continue() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::FeedbackChoice(true)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::FeedbackChoice(proceed) => assert!(proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_orchestrator_feedback_choice_retry() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::FeedbackChoice(false)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::FeedbackChoice(proceed) => assert!(!proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn read_server_orc_msg_feedback_choice_continue() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::FeedbackChoice(true)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        match msg {
            ServerOrcMsg::FeedbackChoice(proceed) => assert!(proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn read_server_orc_msg_feedback_choice_retry() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::FeedbackChoice(false)).unwrap();

        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        match msg {
            ServerOrcMsg::FeedbackChoice(proceed) => assert!(!proceed),
            other => panic!("Expected FeedbackChoice, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_client_summary_request() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::SummaryRequest).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ClientMsg::SummaryRequest));
    }

    #[test]
    fn round_trip_session_summary() {
        let text = "## Session Summary\n\n### Key Vocabulary\n- word: definition".to_string();
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::SessionSummary(text.clone())).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::SessionSummary(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected SessionSummary, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_orchestrator_summary_request() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SummaryRequest).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        assert!(matches!(msg, OrchestratorMsg::SummaryRequest));
    }

    #[test]
    fn round_trip_orchestrator_summary_response() {
        let text = "# Summary\nContent here".to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SummaryResponse(text.clone())).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::SummaryResponse(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected SummaryResponse, got {other:?}"),
        }
    }

    #[test]
    fn read_server_orc_msg_summary_request() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::SummaryRequest).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerOrcMsg::SummaryRequest));
    }

    #[test]
    fn is_disconnect_detects_unexpected_eof() {
        let err = anyhow::Error::new(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "connection closed",
        ));
        assert!(super::is_disconnect(&err));
    }

    #[test]
    fn is_disconnect_detects_broken_pipe() {
        let err = anyhow::Error::new(std::io::Error::new(ErrorKind::BrokenPipe, "pipe broke"));
        assert!(super::is_disconnect(&err));
    }

    #[test]
    fn is_disconnect_detects_connection_reset() {
        let err = anyhow::Error::new(std::io::Error::new(
            ErrorKind::ConnectionReset,
            "peer reset",
        ));
        assert!(super::is_disconnect(&err));
    }

    #[test]
    fn is_disconnect_ignores_other_errors() {
        let err = anyhow::anyhow!("some other error");
        assert!(!super::is_disconnect(&err));
    }

    // --- StatusNotification tests ---

    #[test]
    fn round_trip_server_status_notification() {
        let text = "Thinking...".to_string();
        let mut buf = Vec::new();
        write_server_msg(&mut buf, &ServerMsg::StatusNotification(text.clone())).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        match msg {
            ServerMsg::StatusNotification(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected StatusNotification, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_cancel_exchange_client() {
        let mut buf = Vec::new();
        write_client_msg(&mut buf, &ClientMsg::CancelExchange).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_client_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ClientMsg::CancelExchange));
    }

    #[test]
    fn round_trip_cancel_exchange_orchestrator() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::CancelExchange).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        assert!(matches!(msg, OrchestratorMsg::CancelExchange));
    }

    #[test]
    fn read_server_orc_msg_cancel_exchange() {
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::CancelExchange).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_server_orc_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerOrcMsg::CancelExchange));
    }

    #[test]
    fn round_trip_orchestrator_status_notification() {
        let text = "Searching the web...".to_string();
        let mut buf = Vec::new();
        write_orchestrator_msg(&mut buf, &OrchestratorMsg::StatusNotification(text.clone()))
            .unwrap();
        let mut cursor = Cursor::new(buf);
        let msg = read_orchestrator_msg(&mut cursor).unwrap();
        match msg {
            OrchestratorMsg::StatusNotification(decoded) => assert_eq!(decoded, text),
            other => panic!("Expected StatusNotification, got {other:?}"),
        }
    }
}
