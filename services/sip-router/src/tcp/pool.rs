use std::{collections::HashMap, net::SocketAddr, time::Duration};

use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc, task::JoinHandle};

use super::{is_response, BoxError, SipFrameReader};
use crate::{
    discovery::SipNode,
    proxy::{remove_top_via, top_via_branch},
};

struct BackendConnection {
    sender: mpsc::Sender<Vec<u8>>,
    task: JoinHandle<()>,
}

/// 单个客户端连接使用的 sip-edge 后端连接池。
pub(super) struct BackendPool {
    connections: HashMap<SocketAddr, BackendConnection>,
    client_sender: mpsc::Sender<Vec<u8>>,
    queue_capacity: usize,
    connect_timeout: Duration,
}

impl BackendPool {
    pub(super) fn new(
        client_sender: mpsc::Sender<Vec<u8>>,
        queue_capacity: usize,
        connect_timeout: Duration,
    ) -> Self {
        Self {
            connections: HashMap::new(),
            client_sender,
            queue_capacity,
            connect_timeout,
        }
    }

    pub(super) async fn send(&mut self, node: &SipNode, frame: Vec<u8>) -> Result<(), BoxError> {
        if let Some(connection) = self.connections.get(&node.address) {
            match connection.sender.try_send(frame) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err("sip-edge TCP 写队列已满".into());
                }
                Err(mpsc::error::TrySendError::Closed(frame)) => {
                    self.remove(node.address);
                    return self.connect_and_send(node, frame).await;
                }
            }
        }
        self.connect_and_send(node, frame).await
    }

    async fn connect_and_send(&mut self, node: &SipNode, frame: Vec<u8>) -> Result<(), BoxError> {
        let stream = tokio::time::timeout(self.connect_timeout, TcpStream::connect(node.address))
            .await
            .map_err(|_| "连接 sip-edge TCP 节点超时")??;
        let (sender, receiver) = mpsc::channel(self.queue_capacity);
        let client_sender = self.client_sender.clone();
        let node_id = node.id.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = run_backend(stream, receiver, client_sender).await {
                tracing::debug!(node_id, %error, "sip-edge TCP 后端连接已关闭");
            }
        });
        sender
            .try_send(frame)
            .map_err(|_| "新建 sip-edge TCP 写队列不可用")?;
        self.connections
            .insert(node.address, BackendConnection { sender, task });
        Ok(())
    }

    fn remove(&mut self, address: SocketAddr) {
        if let Some(connection) = self.connections.remove(&address) {
            connection.task.abort();
        }
    }
}

impl Drop for BackendPool {
    fn drop(&mut self) {
        for (_, connection) in self.connections.drain() {
            connection.task.abort();
        }
    }
}

async fn run_backend(
    stream: TcpStream,
    mut outbound: mpsc::Receiver<Vec<u8>>,
    client_sender: mpsc::Sender<Vec<u8>>,
) -> Result<(), BoxError> {
    let (read, mut write) = stream.into_split();
    let mut reader = SipFrameReader::new(read);
    let writer = async {
        while let Some(frame) = outbound.recv().await {
            write.write_all(&frame).await?;
        }
        Ok::<(), BoxError>(())
    };
    let receiver = async {
        while let Some(frame) = reader.read_frame().await? {
            let forwarded = if is_response(&frame)
                && top_via_branch(&frame)
                    .is_some_and(|branch| branch.starts_with("z9hG4bK-vosrs-tcp-"))
            {
                let mut output = Vec::new();
                remove_top_via(&frame, &mut output)?;
                output
            } else {
                frame
            };
            client_sender
                .try_send(forwarded)
                .map_err(|_| "客户端 TCP 写队列已满或已关闭")?;
        }
        Ok::<(), BoxError>(())
    };
    tokio::try_join!(writer, receiver)?;
    Ok(())
}
