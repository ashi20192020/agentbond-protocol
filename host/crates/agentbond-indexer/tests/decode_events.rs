use agentbond_db::Commitment;
use agentbond_indexer::extract_protocol_events;
use agentbond_sdk::program_id;
use agentbond_types::{EVENT_ENCODED_LEN, ProtocolEvent, ProtocolEventKind};
use base64::Engine;

fn b64(event: &ProtocolEvent) -> String {
    Engine::encode(&base64::engine::general_purpose::STANDARD, event.encode())
}

#[test]
fn every_event_kind_round_trips() {
    let program = program_id();
    for kind_u8 in 1u8..=17 {
        let kind = ProtocolEventKind::from_u8(kind_u8).expect("kind");
        let event = ProtocolEvent {
            kind,
            subject: [3u8; 32],
            actor: [4u8; 32],
            amount: u64::MAX,
            timestamp: 1_700_000_000,
        };
        let logs = vec![format!("Program data: {}", b64(&event))];
        let out = extract_protocol_events(&program, "sig", 9, &logs, Commitment::Processed)
            .expect("extract");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, kind_u8);
        assert_eq!(out[0].amount, u64::MAX);
        assert_eq!(out[0].event_index, 0);
    }
}

#[test]
fn golden_82_byte_vector_and_multi_event_order() {
    let program = program_id();
    let a = ProtocolEvent {
        kind: ProtocolEventKind::JobCreated,
        subject: [1u8; 32],
        actor: [2u8; 32],
        amount: 10,
        timestamp: 11,
    };
    let b = ProtocolEvent {
        kind: ProtocolEventKind::JobFunded,
        subject: [1u8; 32],
        actor: [2u8; 32],
        amount: 10,
        timestamp: 12,
    };
    assert_eq!(a.encode().len(), EVENT_ENCODED_LEN);
    let logs = vec![
        "unrelated".into(),
        format!("Program data: {}", b64(&a)),
        format!("Program data: {}", b64(&b)),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Finalized).expect("extract");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].event_index, 0);
    assert_eq!(out[1].event_index, 1);
    assert_eq!(out[0].kind, ProtocolEventKind::JobCreated.as_u8());
    assert_eq!(out[1].kind, ProtocolEventKind::JobFunded.as_u8());
}

#[test]
fn malformed_and_wrong_length_ignored() {
    let program = program_id();
    let logs = vec![
        "Program data: %%%".into(),
        format!(
            "Program data: {}",
            Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8; 10])
        ),
        format!(
            "Program data: {}",
            Engine::encode(&base64::engine::general_purpose::STANDARD, [9u8; 82])
        ),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(out.is_empty());
}

#[test]
fn arbitrary_log_bytes_never_panic() {
    let program = program_id();
    for n in 0..120 {
        let junk = Engine::encode(&base64::engine::general_purpose::STANDARD, vec![n as u8; n]);
        let logs = vec![format!("Program data: {junk}")];
        let _ = extract_protocol_events(&program, "s", 1, &logs, Commitment::Processed);
    }
}

#[test]
fn wrong_program_context_still_parses_program_data_prefix() {
    // Extraction is scoped by configured program in the caller; malformed kinds are dropped.
    let program = program_id();
    let mut bad = [0u8; 82];
    bad[0] = 99; // unknown version/kind layout for ProtocolEvent::decode
    let logs = vec![format!(
        "Program data: {}",
        Engine::encode(&base64::engine::general_purpose::STANDARD, bad)
    )];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(out.is_empty());
}
