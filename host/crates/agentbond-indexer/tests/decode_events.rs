use agentbond_db::Commitment;
use agentbond_indexer::extract_protocol_events;
use agentbond_sdk::program_id;
use agentbond_types::{EVENT_ENCODED_LEN, ProtocolEvent, ProtocolEventKind};
use base64::Engine;
use solana_pubkey::Pubkey;

fn b64(event: &ProtocolEvent) -> String {
    Engine::encode(&base64::engine::general_purpose::STANDARD, event.encode())
}

fn program_data(event: &ProtocolEvent) -> String {
    format!("Program data: {}", b64(event))
}

/// Wrap inner logs with a top-level invoke / success frame for `program`.
fn with_invoke(program: &Pubkey, inner: impl IntoIterator<Item = String>) -> Vec<String> {
    let id = program.to_string();
    let mut logs = vec![format!("Program {id} invoke [1]")];
    logs.extend(inner);
    logs.push(format!("Program {id} success"));
    logs
}

fn sample_event(kind: ProtocolEventKind, amount: u64, timestamp: i64) -> ProtocolEvent {
    ProtocolEvent {
        kind,
        subject: [3u8; 32],
        actor: [4u8; 32],
        amount,
        timestamp,
    }
}

fn other_program() -> Pubkey {
    Pubkey::new_from_array([9u8; 32])
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
        let logs = with_invoke(&program, [program_data(&event)]);
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
    let logs = with_invoke(
        &program,
        ["unrelated".into(), program_data(&a), program_data(&b)],
    );
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
    let logs = with_invoke(
        &program,
        [
            "Program data: %%%".into(),
            format!(
                "Program data: {}",
                Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8; 10])
            ),
            format!(
                "Program data: {}",
                Engine::encode(&base64::engine::general_purpose::STANDARD, [9u8; 82])
            ),
        ],
    );
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(out.is_empty());
}

#[test]
fn arbitrary_log_bytes_never_panic() {
    let program = program_id();
    for n in 0..120 {
        let junk = Engine::encode(&base64::engine::general_purpose::STANDARD, vec![n as u8; n]);
        let framed = with_invoke(&program, [format!("Program data: {junk}")]);
        let _ = extract_protocol_events(&program, "s", 1, &framed, Commitment::Processed);
        let bare = vec![
            format!("Program data: {junk}"),
            "Program totally-not-a-frame".into(),
            format!("Program {junk} invoke [1]"),
            format!("Program {junk} success"),
            format!("Program {junk} failed"),
        ];
        let _ = extract_protocol_events(&program, "s", 1, &bare, Commitment::Processed);
    }
}

#[test]
fn agentbond_toplevel_event_accepted() {
    let program = program_id();
    let event = sample_event(ProtocolEventKind::JobCreated, 42, 99);
    let logs = with_invoke(&program, [program_data(&event)]);
    let out =
        extract_protocol_events(&program, "sig", 7, &logs, Commitment::Processed).expect("extract");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ProtocolEventKind::JobCreated.as_u8());
    assert_eq!(out[0].amount, 42);
    assert_eq!(out[0].event_timestamp, 99);
    assert_eq!(out[0].program_id, program.to_bytes());
}

#[test]
fn nested_cpi_event_ignored() {
    let program = program_id();
    let other = other_program();
    let nested = sample_event(ProtocolEventKind::JobCreated, 1, 10);
    let outer = sample_event(ProtocolEventKind::JobFunded, 2, 20);
    let logs = vec![
        format!("Program {} invoke [1]", program),
        format!("Program {} invoke [2]", other),
        program_data(&nested),
        format!("Program {} success", other),
        program_data(&outer),
        format!("Program {} success", program),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ProtocolEventKind::JobFunded.as_u8());
    assert_eq!(out[0].amount, 2);
    assert_eq!(out[0].event_index, 0);
}

#[test]
fn agentbond_before_and_after_cpi() {
    let program = program_id();
    let other = other_program();
    let before = sample_event(ProtocolEventKind::JobCreated, 10, 11);
    let fake = sample_event(ProtocolEventKind::JobFunded, 99, 12);
    let after = sample_event(ProtocolEventKind::ReceiptSubmitted, 20, 13);
    let logs = vec![
        format!("Program {} invoke [1]", program),
        program_data(&before),
        format!("Program {} invoke [2]", other),
        program_data(&fake),
        format!("Program {} success", other),
        program_data(&after),
        format!("Program {} success", program),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].kind, ProtocolEventKind::JobCreated.as_u8());
    assert_eq!(out[0].amount, 10);
    assert_eq!(out[0].event_index, 0);
    assert_eq!(out[1].kind, ProtocolEventKind::ReceiptSubmitted.as_u8());
    assert_eq!(out[1].amount, 20);
    assert_eq!(out[1].event_index, 1);
}

#[test]
fn wrong_toplevel_program() {
    let program = program_id();
    let other = other_program();
    let event = sample_event(ProtocolEventKind::JobCreated, 7, 8);
    let logs = with_invoke(&other, [program_data(&event)]);
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(out.is_empty());
}

#[test]
fn malformed_stack() {
    let program = program_id();
    let event = sample_event(ProtocolEventKind::JobCreated, 1, 2);
    let logs = vec![
        format!("Program {} success", program),
        program_data(&event),
        format!("Program {} failed", program),
        program_data(&event),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(out.is_empty());
}

#[test]
fn mismatched_close_clears_stack_and_ignores_event() {
    let program = program_id();
    let other = other_program();
    let event = sample_event(ProtocolEventKind::JobCreated, 42, 99);
    // AgentBond invoke → CPI invoke → mismatched close → valid event bytes.
    let logs = vec![
        format!("Program {} invoke [1]", program),
        format!("Program {} invoke [2]", other),
        format!("Program {} success", program), // mismatch: closes wrong id
        program_data(&event),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert!(
        out.is_empty(),
        "mismatched close must clear stack and ignore later events"
    );
}

#[test]
fn failed_invocation() {
    let program = program_id();
    let other = other_program();
    let during_agentbond = sample_event(ProtocolEventKind::JobCreated, 1, 10);
    let after_fail = sample_event(ProtocolEventKind::JobFunded, 2, 20);
    let after_nested_fail = sample_event(ProtocolEventKind::ReceiptSubmitted, 3, 30);

    // AgentBond failed pops its frame; subsequent Program data must not be attributed.
    let logs = vec![
        format!("Program {} invoke [1]", program),
        program_data(&during_agentbond),
        format!("Program {} failed", program),
        program_data(&after_fail),
    ];
    let out =
        extract_protocol_events(&program, "sig", 1, &logs, Commitment::Processed).expect("extract");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ProtocolEventKind::JobCreated.as_u8());
    assert_eq!(out[0].amount, 1);

    // Nested CPI failure pops only the nested frame; AgentBond remains active.
    let logs = vec![
        format!("Program {} invoke [1]", program),
        format!("Program {} invoke [2]", other),
        program_data(&after_fail),
        format!("Program {} failed", other),
        program_data(&after_nested_fail),
        format!("Program {} success", program),
    ];
    let out =
        extract_protocol_events(&program, "sig", 2, &logs, Commitment::Processed).expect("extract");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ProtocolEventKind::ReceiptSubmitted.as_u8());
    assert_eq!(out[0].amount, 3);
}
