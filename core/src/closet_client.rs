use anyhow::{anyhow, Result};
use iroh::{Endpoint, EndpointAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{ClosetRequest, ClosetResponse, CLOSET_ALPN};

const MAX_CLOSET_RESPONSE_FRAME_LEN: usize = 512 * 1024;

pub async fn send_closet_request(
    endpoint: &Endpoint,
    endpoint_addr: EndpointAddr,
    request: ClosetRequest,
) -> Result<ClosetResponse> {
    let connection = endpoint
        .connect(endpoint_addr, CLOSET_ALPN)
        .await
        .map_err(|e| anyhow!("closet endpoint.connect() failed: {e}"))?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| anyhow!("closet connection.open_bi() failed: {e}"))?;

    let payload = serde_json::to_vec(&request)?;
    send.write_u32(payload.len() as u32).await?;
    send.write_all(&payload).await?;
    send.flush().await?;

    let frame_len = recv.read_u32().await? as usize;
    if frame_len > MAX_CLOSET_RESPONSE_FRAME_LEN {
        return Err(anyhow!("closet response frame too large: {frame_len}"));
    }

    let mut bytes = vec![0u8; frame_len];
    recv.read_exact(&mut bytes).await?;

    let _ = send.finish();
    connection.close(0u32.into(), b"ok");

    serde_json::from_slice::<ClosetResponse>(&bytes).map_err(Into::into)
}

pub async fn closet_start(endpoint: &Endpoint, endpoint_addr: EndpointAddr) -> Result<ClosetResponse> {
    send_closet_request(endpoint, endpoint_addr, ClosetRequest::Start).await
}

pub async fn closet_command(
    endpoint: &Endpoint,
    endpoint_addr: EndpointAddr,
    session_id: impl Into<String>,
    input: impl Into<String>,
) -> Result<ClosetResponse> {
    send_closet_request(
        endpoint,
        endpoint_addr,
        ClosetRequest::Command {
            session_id: session_id.into(),
            input: input.into(),
        },
    )
    .await
}

pub async fn closet_submit_citizenship(
    endpoint: &Endpoint,
    endpoint_addr: EndpointAddr,
    session_id: impl Into<String>,
    ipns_private_key_base64: impl Into<String>,
    desired_fragment: Option<String>,
) -> Result<ClosetResponse> {
    send_closet_request(
        endpoint,
        endpoint_addr,
        ClosetRequest::SubmitCitizenship {
            session_id: session_id.into(),
            ipns_private_key_base64: ipns_private_key_base64.into(),
            desired_fragment,
        },
    )
    .await
}
