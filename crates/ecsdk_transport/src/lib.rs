use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

// ---------------------------------------------------------------------------
// Packet type — [channel_id: u8][data...] inside a length-delimited frame
// ---------------------------------------------------------------------------

pub struct RepliconPacket {
    pub channel_id: u8,
    pub data: Vec<u8>,
}

impl RepliconPacket {
    pub fn encode(&self) -> Bytes {
        let mut buf = Vec::with_capacity(1 + self.data.len());
        buf.push(self.channel_id);
        buf.extend_from_slice(&self.data);
        buf.into()
    }

    pub fn decode(frame: Bytes) -> Option<Self> {
        if frame.is_empty() {
            return None;
        }
        Some(Self {
            channel_id: frame[0],
            data: frame[1..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// Bidirectional bridge: framed stream ↔ mpsc channels
// ---------------------------------------------------------------------------

pub async fn run_bridge(
    stream: impl AsyncRead + AsyncWrite + Send + Unpin,
    to_remote_rx: &mut mpsc::UnboundedReceiver<RepliconPacket>,
    from_remote_tx: &mpsc::UnboundedSender<RepliconPacket>,
    wake: impl Fn(),
) {
    let (mut sink, mut source) = Framed::new(stream, LengthDelimitedCodec::new()).split();

    let send_to_remote = async {
        while let Some(packet) = to_remote_rx.recv().await {
            if sink.send(packet.encode()).await.is_err() {
                break;
            }
        }
    };

    let recv_from_remote = async {
        while let Some(Ok(frame)) = source.next().await {
            if let Some(packet) = RepliconPacket::decode(frame.into()) {
                let _ = from_remote_tx.send(packet);
                wake();
            }
        }
    };

    tokio::select! {
        _ = send_to_remote => {}
        _ = recv_from_remote => {}
    }
}
