//! GNET-03: manual two-computer Connect Four transport lab.
//! Only an opt-in command opens a UDP socket; there is no discovery, external
//! relay, pubsub, game save, or access to Akula files.

use akula_game_lan_roha::{
    FRAME_TIMEOUT, RohaLanTransport, accept_lan, bind_lan, connect_lan, next_frame,
};
use akula_game_session::{
    ApplyResult, Command, GameSession, PeerId, RoomId, Status,
    transport::{GameTransport, ReceivedFrame},
    wire::WireMessage,
};
use anyhow::{Result, anyhow, bail, ensure};
use iroh::EndpointId;
use std::{
    io::{self, Write},
    net::SocketAddr,
    str::FromStr,
    time::Duration,
};

const TURNS: [u8; 20] = [3, 4, 6, 1, 0, 1, 0, 0, 3, 3, 3, 0, 5, 5, 5, 2, 1, 3, 2, 3];

fn usage() -> &'static str {
    "Usage:\n  akula-game-lan-roha host <LAN-IP:PORT>\n  akula-game-lan-roha join <LOCAL-LAN-IP:0> <HOST-LAN-IP:PORT> <HOST-ENDPOINT-ID>\n\nNo external DNS/relay/port mapping. Host must manually approve the guest ID.\n"
}

fn peer(id: EndpointId) -> PeerId {
    PeerId(*id.as_bytes())
}

fn wire_error(error: akula_game_session::wire::WireError) -> anyhow::Error {
    anyhow!("invalid game message: {error:?}")
}

async fn receive_from(transport: &mut RohaLanTransport, expected: PeerId) -> Result<WireMessage> {
    let ReceivedFrame { from, payload } = next_frame(transport, FRAME_TIMEOUT).await?;
    ensure!(
        from == expected,
        "game packet from an unexpected authenticated peer"
    );
    WireMessage::decode(&payload).map_err(wire_error)
}

fn send_wire(
    transport: &mut RohaLanTransport,
    target: PeerId,
    message: &WireMessage,
) -> Result<()> {
    let bytes = message.encode().map_err(wire_error)?;
    transport
        .send(target, &bytes)
        .map_err(|e| anyhow!("send queue: {e:?}"))
}

fn send_snapshot(
    transport: &mut RohaLanTransport,
    recipient: PeerId,
    room: RoomId,
    epoch: u64,
    game: &GameSession,
) -> Result<()> {
    let state = game
        .snapshot_for(recipient)
        .map_err(|e| anyhow!("game snapshot: {e:?}"))?
        .state;
    send_wire(
        transport,
        recipient,
        &WireMessage::Snapshot { room, epoch, state },
    )
}

async fn host(bind: SocketAddr) -> Result<()> {
    let endpoint = bind_lan(bind).await?;
    let host_id = endpoint.id();
    let sockets = endpoint.bound_sockets();
    println!("HOST_ID={host_id}");
    for addr in sockets {
        println!("HOST_LAN_ADDR={addr}");
    }
    println!("Waiting for one LAN peer; no external relay or DNS is configured.");
    let conn = accept_lan(&endpoint).await?;
    let requested_id = conn.remote_id();
    println!("Authenticated connection from: {requested_id}");
    println!("Compare with GUEST_ID displayed on the guest computer.");
    print!("Paste the GUEST_ID from that computer to approve: ");
    io::stdout().flush()?;
    let mut approved = String::new();
    io::stdin().read_line(&mut approved)?;
    ensure!(
        approved.trim() == requested_id.to_string(),
        "guest peer was not approved; no game data sent"
    );

    let mut transport = RohaLanTransport::new(&endpoint, conn);
    let host_peer = peer(host_id);
    let guest_peer = peer(requested_id);
    let mut room_bytes: [u8; 16] = rand::random();
    if room_bytes == [0; 16] {
        room_bytes = rand::random();
    }
    let room = RoomId(room_bytes);
    let epoch = 1;
    let mut game = GameSession::new(room, epoch, host_peer, guest_peer)
        .map_err(|e| anyhow!("game creation failed: {e:?}"))?;
    send_snapshot(&mut transport, guest_peer, room, epoch, &game)?;

    for (turn, &column) in TURNS.iter().enumerate() {
        let sequence = (turn / 2 + 1) as u64;
        if turn % 2 == 0 {
            let command = Command {
                room,
                epoch,
                sequence,
                column,
            };
            game.apply(host_peer, command)
                .map_err(|e| anyhow!("host move: {e:?}"))?;
        } else {
            let message = receive_from(&mut transport, guest_peer).await?;
            let WireMessage::Command(command) = message else {
                bail!("host expected guest command");
            };
            ensure!(
                command.sequence == sequence && command.column == column,
                "guest command differs from the agreed test scenario"
            );
            game.apply(guest_peer, command)
                .map_err(|e| anyhow!("guest move: {e:?}"))?;
        }
        send_snapshot(&mut transport, guest_peer, room, epoch, &game)?;
    }
    let ack = receive_from(&mut transport, guest_peer).await?;
    let WireMessage::Command(command) = ack else {
        bail!("expected duplicate guest command as final acknowledgement");
    };
    ensure!(
        command.sequence == 10 && command.column == TURNS[19],
        "invalid final guest acknowledgement"
    );
    ensure!(
        game.apply(guest_peer, command)
            == Ok(ApplyResult::Duplicate {
                original_revision: 20
            }),
        "final command was not recognized as duplicate"
    );
    send_snapshot(&mut transport, guest_peer, room, epoch, &game)?;
    println!("HOST PASS: 20 authoritative turns; final duplicate ignored; revision 20.");
    // The remote sends one final duplicate after receiving the echo, proving
    // the last snapshot actually reached the guest before we close.
    let end = receive_from(&mut transport, guest_peer).await?;
    let WireMessage::Command(cmd) = end else {
        bail!("expected final close ack");
    };
    ensure!(
        game.apply(guest_peer, cmd)
            == Ok(ApplyResult::Duplicate {
                original_revision: 20
            }),
        "invalid final close acknowledgement"
    );
    drop(transport);
    endpoint.close().await;
    Ok(())
}

async fn guest(bind: SocketAddr, host_addr: SocketAddr, host_id: EndpointId) -> Result<()> {
    let endpoint = bind_lan(bind).await?;
    println!("GUEST_ID={}", endpoint.id());
    println!("Tell the host operator this exact ID for manual approval.");
    let conn = connect_lan(&endpoint, host_id, host_addr).await?;
    let mut transport = RohaLanTransport::new(&endpoint, conn);
    let host_peer = peer(host_id);

    let initial = receive_from(&mut transport, host_peer).await?;
    let WireMessage::Snapshot {
        room,
        epoch,
        state: first,
    } = initial
    else {
        bail!("expected initial board snapshot");
    };
    ensure!(first.revision == 0, "initial board revision must be zero");
    for (turn, &column) in TURNS.iter().enumerate() {
        if turn % 2 == 1 {
            let command = Command {
                room,
                epoch,
                sequence: (turn / 2 + 1) as u64,
                column,
            };
            send_wire(&mut transport, host_peer, &WireMessage::Command(command))?;
        }
        let snapshot = receive_from(&mut transport, host_peer).await?;
        let WireMessage::Snapshot {
            room: got_room,
            epoch: got_epoch,
            state,
        } = snapshot
        else {
            bail!("expected host snapshot");
        };
        ensure!(
            got_room == room && got_epoch == epoch,
            "wrong room in snapshot"
        );
        ensure!(
            state.revision == (turn + 1) as u64,
            "unexpected snapshot revision"
        );
        if turn == 19 {
            ensure!(
                state.status
                    == Status::InProgress {
                        next: akula_game_session::Player::Red
                    },
                "unexpected final state"
            );
        }
    }
    let finish = Command {
        room,
        epoch,
        sequence: 10,
        column: TURNS[19],
    };
    send_wire(&mut transport, host_peer, &WireMessage::Command(finish))?;
    let last = receive_from(&mut transport, host_peer).await?;
    let WireMessage::Snapshot { state, .. } = last else {
        bail!("expected duplicate acknowledgement snapshot");
    };
    ensure!(state.revision == 20, "final revision mismatch");
    println!("GUEST PASS: received matching snapshots for all 20 moves.");
    send_wire(&mut transport, host_peer, &WireMessage::Command(finish))?;
    // Give the async writer a bounded chance to submit final closure proof.
    tokio::time::sleep(Duration::from_millis(250)).await;
    drop(transport);
    endpoint.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode, bind] if mode == "host" => host(bind.parse()?).await,
        [mode, bind, host_addr, host_id] if mode == "join" => {
            guest(
                bind.parse()?,
                host_addr.parse()?,
                EndpointId::from_str(host_id)?,
            )
            .await
        }
        _ => bail!("{}", usage()),
    }
}
