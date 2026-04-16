use super::*;

impl WorldProtocol {
    pub(crate) fn content_type_matches(actual: &str, canonical: &str, legacy: &str) -> bool {
        actual == canonical || actual == legacy
    }

    pub(crate) async fn room_signing_key(&self, room_url: &str) -> Result<SigningKey> {
        let room_url_parsed = Did::try_from(room_url)
            .map_err(|e| anyhow!("invalid room did '{}': {}", room_url, e))?;
        let room_url_canonical = room_url_parsed.id();
        let signing_did = Did::new_root(&room_url_parsed.ipns)
            .map_err(|e| anyhow!("invalid signing did for room {}: {}", room_url, e))?;

        if let Some(room_key) = {
            let slots = self.world.actor_secrets.read().await;
            slots
                .get(room_url)
                .or_else(|| slots.get(room_url_canonical.as_str()))
                .map(|secret| secret.signing_key)
        } {
            return SigningKey::from_private_key_bytes(signing_did.clone(), room_key)
                .map_err(|e| anyhow!("failed to restore signing key for room {}: {}", room_url, e));
        }

        if let Some(world_key) = {
            let world_key_guard = self.world.unlocked_world_signing_key.read().await;
            *world_key_guard
        } {
            return SigningKey::from_private_key_bytes(signing_did, world_key)
                .map_err(|e| anyhow!("failed to restore fallback signing key for room {}: {}", room_url, e));
        }

        Err(anyhow!(
            "missing signing key for room {}: missing room actor secret and missing unlocked world signing key",
            room_url
        ))
    }

    pub(crate) async fn room_presence_context(
        &self,
        room_name: &str,
    ) -> Result<(String, String, String, Vec<PresenceAvatar>, Vec<String>)> {
        let rooms = self.world.rooms.read().await;
        let room = rooms
            .get(room_name)
            .ok_or_else(|| anyhow!("room '{}' not found", room_name))?;

        let mut avatars = Vec::new();
        let mut endpoints = Vec::new();
        for (handle, avatar) in &room.avatars {
            avatars.push(PresenceAvatar {
                handle: handle.clone(),
                url: avatar.agent_did.id(),
                identity: avatar.identity.clone(),
            });
            endpoints.push(avatar.agent_endpoint.clone());
        }
        avatars.sort_by(|a, b| a.handle.cmp(&b.handle));
        endpoints.sort();
        endpoints.dedup();

        Ok((
            room.url.clone(),
            room.title_or_default(),
            room.description_or_default(),
            avatars,
            endpoints,
        ))
    }

    pub(crate) fn push_cache_key(target_endpoint_id: &str, lane_alpn: &'static [u8]) -> String {
        format!("{}|{}", String::from_utf8_lossy(lane_alpn), target_endpoint_id.trim())
    }

    pub(crate) async fn create_push_stream_cache(
        &self,
        target_endpoint_id: &str,
        lane_alpn: &'static [u8],
    ) -> Result<PushStreamCache> {
        let target: EndpointId = target_endpoint_id
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid target endpoint id {}: {}", target_endpoint_id, e))?;

        let relay: RelayUrl = DEFAULT_WORLD_RELAY_URL
            .parse()
            .map_err(|e| anyhow!("invalid relay URL {}: {}", DEFAULT_WORLD_RELAY_URL, e))?;
        let endpoint_addr = EndpointAddr::new(target).with_relay_url(relay);

        let connection = self
            .endpoint
            .connect(endpoint_addr, lane_alpn)
            .await
            .map_err(|e| anyhow!("push endpoint.connect failed: {}", e))?;

        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|e| anyhow!("push connection.open_bi failed: {}", e))?;

        Ok(PushStreamCache {
            connection,
            send,
            recv,
        })
    }

    pub(crate) async fn exchange_push_on_stream(
        &self,
        cache: &mut PushStreamCache,
        message_cbor: Vec<u8>,
    ) -> Result<()> {
        let request = OutboxRequest::Signed { message_cbor };
        let payload = serde_json::to_vec(&request)
            .map_err(|e| anyhow!("failed to serialize outbox request: {}", e))?;

        cache
            .send
            .write_u32(payload.len() as u32)
            .await
            .map_err(|e| anyhow!("push write_u32 failed: {}", e))?;
        cache
            .send
            .write_all(&payload)
            .await
            .map_err(|e| anyhow!("push write_all failed: {}", e))?;
        cache
            .send
            .flush()
            .await
            .map_err(|e| anyhow!("push flush failed: {}", e))?;

        let frame_len = cache
            .recv
            .read_u32()
            .await
            .map_err(|e| anyhow!("push read_u32 failed: {}", e))? as usize;
        if frame_len > 256 * 1024 {
            return Err(anyhow!("push response frame too large: {}", frame_len));
        }

        let mut bytes = vec![0u8; frame_len];
        cache
            .recv
            .read_exact(&mut bytes)
            .await
            .map_err(|e| anyhow!("push read_exact failed: {}", e))?;
        let response: OutboxResponse = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow!("push response decode failed: {}", e))?;
        if !response.ok {
            return Err(anyhow!("push rejected: {}", response.message));
        }

        Ok(())
    }

    pub(crate) async fn send_signed_push_to_endpoint_on_lane(
        &self,
        target_endpoint_id: &str,
        message_cbor: Vec<u8>,
        lane_alpn: &'static [u8],
    ) -> Result<()> {
        let cache_key = Self::push_cache_key(target_endpoint_id, lane_alpn);
        let now = Instant::now();
        {
            let mut cooldowns = self.push_timeout_cooldown.lock().await;
            cooldowns.retain(|_, until| *until > now);
            if let Some(until) = cooldowns.get(&cache_key) {
                debug!(
                    "push endpoint {} on lane {} is in cooldown for {:?}",
                    target_endpoint_id,
                    String::from_utf8_lossy(lane_alpn),
                    until.saturating_duration_since(now)
                );
                return Ok(());
            }
        }
        let mut last_error: Option<anyhow::Error> = None;

        for _ in 0..2 {
            let mut stream_cache = {
                let mut slots = self.push_stream_cache.lock().await;
                slots.remove(&cache_key)
            }
            .unwrap_or(self.create_push_stream_cache(target_endpoint_id, lane_alpn).await?);

            match self
                .exchange_push_on_stream(&mut stream_cache, message_cbor.clone())
                .await
            {
                Ok(()) => {
                    let mut slots = self.push_stream_cache.lock().await;
                    slots.insert(cache_key.clone(), stream_cache);
                    return Ok(());
                }
                Err(err) => {
                    if err.to_string().contains("timed out") {
                        let until = Instant::now() + Duration::from_secs(PUSH_TIMEOUT_COOLDOWN_SECS);
                        let mut cooldowns = self.push_timeout_cooldown.lock().await;
                        cooldowns.insert(cache_key.clone(), until);
                        warn!(
                            "push timeout for endpoint {} on lane {}; cooling down for {}s",
                            target_endpoint_id,
                            String::from_utf8_lossy(lane_alpn),
                            PUSH_TIMEOUT_COOLDOWN_SECS
                        );
                        stream_cache.connection.close(0u32.into(), b"push timeout");
                        return Ok(());
                    }
                    last_error = Some(err);
                    stream_cache.connection.close(0u32.into(), b"stream error");
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("push failed")))
    }

    pub(crate) async fn send_signed_push_to_endpoint(
        &self,
        target_endpoint_id: &str,
        message_cbor: Vec<u8>,
    ) -> Result<()> {
        self.send_signed_push_to_endpoint_on_lane(target_endpoint_id, message_cbor, PRESENCE_ALPN)
            .await
    }

    pub(crate) async fn push_presence_snapshot_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_url, room_title, room_description, avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence snapshot context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_url).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence snapshot signing key unavailable for {}: {}", room_url, err);
                return;
            }
        };
        let seq = self
            .world
            .latest_room_event_sequence(room_name)
            .await
            .unwrap_or(0);

        let payload = PresenceSnapshotEvent {
            v: 1,
            kind: "presence.snapshot".to_string(),
            room: room_name.to_string(),
            room_url: room_url.clone(),
            room_title,
            room_description,
            avatars,
            seq,
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence snapshot encode failed for room '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_url.clone(),
            room_url,
            CONTENT_TYPE_PRESENCE,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("presence snapshot message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence snapshot cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence snapshot push to {} failed: {}", target_endpoint_id, err);
        }
    }

    pub(crate) async fn push_presence_snapshot(&self, room_name: &str) {
        let context = self.room_presence_context(room_name).await;
        let (_room_url, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence snapshot context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        for endpoint in endpoints {
            self.push_presence_snapshot_to(room_name, &endpoint).await;
        }
    }

    pub(crate) async fn push_presence_room_state_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_url, room_title, room_description, avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence room-state context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        let signing_key = match self.room_signing_key(&room_url).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence room-state signing key unavailable for {}: {}", room_url, err);
                return;
            }
        };

        let latest_event_sequence = self
            .world
            .latest_room_event_sequence(room_name)
            .await
            .unwrap_or(0);
        let room_object_dids = self.world.room_object_did_map(room_name).await;

        let payload = PresenceRoomStateEvent {
            v: 1,
            kind: "presence.room_state".to_string(),
            room: room_name.to_string(),
            room_url: room_url.clone(),
            room_title,
            room_description,
            avatars,
            latest_event_sequence,
            room_object_dids,
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence room-state encode failed for room '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_url.clone(),
            room_url,
            CONTENT_TYPE_PRESENCE,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("presence room-state message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence room-state cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence room-state push to {} failed: {}", target_endpoint_id, err);
        }
    }

    pub(crate) async fn push_room_events(&self, room_name: &str, since_sequence: u64) {
        let context = self.room_presence_context(room_name).await;
        let (room_url, room_title, room_description, avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("room events context unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_url).await {
            Ok(key) => key,
            Err(err) => {
                warn!("room event signing key unavailable for {}: {}", room_url, err);
                return;
            }
        };
        let (events, latest_event_sequence) = match self.world.room_events_since(room_name, since_sequence).await {
            Ok(val) => val,
            Err(err) => {
                warn!("room events unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        if events.is_empty() {
            return;
        }

        for event in events {
            let payload = RoomEventEnvelope {
                v: 1,
                kind: "room.event".to_string(),
                room: room_name.to_string(),
                room_url: room_url.clone(),
                room_title: room_title.clone(),
                room_description: room_description.clone(),
                avatars: avatars.clone(),
                event,
                latest_event_sequence,
                ts: Utc::now().to_rfc3339(),
            };
            let content = match serde_json::to_vec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("room event encode failed for '{}': {}", room_name, err);
                    continue;
                }
            };
            let message = match Message::new(
                room_url.clone(),
                room_url.clone(),
                CONTENT_TYPE_EVENT,
                content,
                &signing_key,
            ) {
                Ok(msg) => msg,
                Err(err) => {
                    warn!("room event message build failed: {}", err);
                    continue;
                }
            };
            let cbor = match message.to_cbor() {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("room event cbor encode failed: {}", err);
                    continue;
                }
            };

            for endpoint in &endpoints {
                let cbor_clone = cbor.clone();
                if let Err(err) = self
                    .send_signed_push_to_endpoint_on_lane(endpoint, cbor_clone, PRESENCE_ALPN)
                    .await
                {
                    warn!("room event push to {} failed: {}", endpoint, err);
                }
            }
        }
    }

    pub(crate) async fn push_world_broadcast(&self, room_name: &str, message_text: &str) {
        let text = message_text.trim();
        if text.is_empty() {
            return;
        }

        let context = self.room_presence_context(room_name).await;
        let (room_url, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("world broadcast context unavailable for '{}': {}", room_name, err);
                return;
            }
        };
        if endpoints.is_empty() {
            return;
        }

        let signing_key = match self.room_signing_key(&room_url).await {
            Ok(key) => key,
            Err(err) => {
                warn!("world broadcast signing key unavailable for {}: {}", room_url, err);
                return;
            }
        };

        let payload = WorldBroadcastEnvelope {
            v: 1,
            kind: "world.broadcast".to_string(),
            room: room_name.to_string(),
            room_url: room_url.clone(),
            message: text.to_string(),
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("world broadcast encode failed for '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_url.clone(),
            room_url,
            CONTENT_TYPE_BROADCAST,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("world broadcast message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("world broadcast cbor encode failed: {}", err);
                return;
            }
        };

        for endpoint in endpoints {
            let cbor_clone = cbor.clone();
            if let Err(err) = self
                .send_signed_push_to_endpoint_on_lane(&endpoint, cbor_clone, INBOX_ALPN)
                .await
            {
                warn!("world broadcast push to {} failed: {}", endpoint, err);
            }
        }
    }

    pub(crate) async fn push_presence_refresh_request_to(
        &self,
        room_name: &str,
        target_endpoint_id: &str,
    ) {
        let context = self.room_presence_context(room_name).await;
        let (room_url, _room_title, _room_description, _avatars, _endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence refresh context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };
        let signing_key = match self.room_signing_key(&room_url).await {
            Ok(key) => key,
            Err(err) => {
                warn!("presence refresh signing key unavailable for {}: {}", room_url, err);
                return;
            }
        };

        let payload = PresenceRefreshRequestEvent {
            v: 1,
            kind: "presence.refresh.request".to_string(),
            room: room_name.to_string(),
            room_url: room_url.clone(),
            ts: Utc::now().to_rfc3339(),
        };
        let content = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence refresh encode failed for room '{}': {}", room_name, err);
                return;
            }
        };
        let message = match Message::new(
            room_url.clone(),
            room_url,
            CONTENT_TYPE_PRESENCE,
            content,
            &signing_key,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!("presence refresh message build failed: {}", err);
                return;
            }
        };
        let cbor = match message.to_cbor() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("presence refresh cbor encode failed: {}", err);
                return;
            }
        };

        if let Err(err) = self.send_signed_push_to_endpoint(target_endpoint_id, cbor).await {
            warn!("presence refresh push to {} failed: {}", target_endpoint_id, err);
        }
    }

    pub(crate) async fn push_presence_refresh_request(&self, room_name: &str) {
        let context = self.room_presence_context(room_name).await;
        let (_room_url, _room_title, _room_description, _avatars, endpoints) = match context {
            Ok(value) => value,
            Err(err) => {
                warn!("presence refresh context unavailable for room '{}': {}", room_name, err);
                return;
            }
        };

        for endpoint in endpoints {
            self.push_presence_refresh_request_to(room_name, &endpoint).await;
        }
    }

    pub(crate) async fn flush_room_dispatch_queue(&self, room_name: &str) {
        let tasks = self.world.drain_room_dispatch_queue(room_name).await;
        for task in tasks {
            match task {
                RoomDispatchTask::PresenceSnapshot => self.push_presence_snapshot(room_name).await,
                RoomDispatchTask::PresenceRoomStateTo(target_endpoint_id) => {
                    self.push_presence_room_state_to(room_name, &target_endpoint_id).await;
                }
                RoomDispatchTask::PresenceRefreshRequest => {
                    self.push_presence_refresh_request(room_name).await;
                }
                RoomDispatchTask::RoomEventsSince(since_sequence) => {
                    self.push_room_events(room_name, since_sequence).await;
                }
                RoomDispatchTask::WorldBroadcast(message_text) => {
                    self.push_world_broadcast(room_name, &message_text).await;
                }
            }
        }
    }

    pub(crate) async fn flush_pending_room_dispatches(&self) {
        let room_names = self.world.room_names().await;
        for room_name in room_names {
            self.flush_room_dispatch_queue(&room_name).await;
        }
    }

    pub(crate) async fn process_request(&self, request: WorldRequest, agent_endpoint: String) -> WorldResponse {
        match self.handle_request(request, agent_endpoint).await {
            Ok(resp) => resp,
            Err(err) => {
                warn!("request error on lane '{}': {}", self.lane.label(), err);
                let detail = err.to_string();
                let ack_code = if detail.contains("does not support this request type") {
                    TransportAckCode::UnsupportedRequestType
                } else if detail.contains("expected ") && detail.contains(" on this lane") {
                    TransportAckCode::InvalidContentType
                } else {
                    TransportAckCode::Rejected
                };

                WorldResponse {
                    ok: false,
                    room: String::new(),
                    message: detail.clone(),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence: 0,
                    broadcasted: false,
                    events: Vec::new(),
                    handle: String::new(),
                    room_description: String::new(),
                    room_title: String::new(),
                    room_url: String::new(),
                    world_did: String::new(),
                    avatars: Vec::new(),
                    room_object_dids: HashMap::new(),
                    transport_ack: Some(TransportAck {
                        lane: self.lane.label().to_string(),
                        code: ack_code,
                        detail,
                    }),
                }
            }
        }
    }

    pub(crate) async fn get_sender_document(&self, sender_root: &Did, force_refresh: bool) -> Result<(Document, bool, bool)> {
        let cache_key = sender_root.ipns.clone();

        if !force_refresh {
            let cache = self.did_cache.read().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok((cached.document.clone(), false, cached.dirty));
            }
        }

        let kubo_url = self.world.kubo_url().await;
        let fetched = kubo::fetch_did_document(&kubo_url, sender_root).await?;

        let existing_dirty = {
            let cache = self.did_cache.read().await;
            cache.get(&cache_key).map(|entry| entry.dirty).unwrap_or(false)
        };

        let mut cache = self.did_cache.write().await;
        cache.insert(
            cache_key,
            CachedDidDocument {
                document: fetched.clone(),
                dirty: existing_dirty,
            },
        );

        Ok((fetched, true, existing_dirty))
    }

    pub(crate) async fn set_sender_dirty(&self, sender_root: &Did, dirty: bool) {
        let cache_key = sender_root.ipns.clone();
        let mut cache = self.did_cache.write().await;
        if let Some(cached) = cache.get_mut(&cache_key) {
            cached.dirty = dirty;
        }
    }

    pub(crate) async fn verify_message(&self, message_cbor: Vec<u8>) -> Result<(Message, Did, Document)> {
        let message = Message::from_cbor(&message_cbor)?;
        let sender_did = Did::try_from(message.from.as_str())?;
        let as_onboarding_did_error = |source: &anyhow::Error| {
            let detail = source.to_string();
            let lowered = detail.to_ascii_lowercase();
            if lowered.contains("failed to fetch did document")
                || lowered.contains("name/resolve failed")
                || lowered.contains("/ipns/")
                || lowered.contains("did document") && lowered.contains("not found")
            {
                anyhow!(
                    "did document is not published yet for {} (submit document via ma/ipfs/1): {}",
                    sender_did.id(),
                    detail
                )
            } else {
                anyhow!(detail)
            }
        };

        let t0 = std::time::Instant::now();
        let (sender_document, fetched_from_kubo, is_dirty) = self.get_sender_document(&sender_did, false).await
            .map_err(|e| {
                warn!("DID doc fetch failed for {} after {:?}: {}", sender_did.id(), t0.elapsed(), e);
                as_onboarding_did_error(&e)
            })?;
        if fetched_from_kubo {
            info!("DID doc for {} fetched via Kubo in {:?}", sender_did.id(), t0.elapsed());
        } else {
            debug!("DID doc for {} served from cache in {:?}", sender_did.id(), t0.elapsed());
        }
        if is_dirty {
            warn!("DID {} is marked dirty; using cached document", sender_did.id());
        }

        if message.verify_with_document(&sender_document).is_ok() {
            if is_dirty {
                self.set_sender_dirty(&sender_did, false).await;
                info!("DID {} cleared from dirty state after successful verification", sender_did.id());
            }
            return Ok((message, sender_did, sender_document));
        }

        warn!(
            "signature verification failed with cached DID doc for {}; retrying fresh fetch",
            sender_did.id()
        );

        let refresh_t0 = std::time::Instant::now();
        let (refreshed_document, refreshed_from_kubo, _) =
            match self.get_sender_document(&sender_did, true).await {
                Ok(value) => value,
                Err(e) => {
                    self.set_sender_dirty(&sender_did, true).await;
                    warn!(
                        "forced DID doc refetch failed for {} after {:?}: {}",
                        sender_did.id(),
                        refresh_t0.elapsed(),
                        e
                    );
                    return Err(as_onboarding_did_error(&e));
                }
            };
        if refreshed_from_kubo {
            info!(
                "DID doc for {} force-fetched via Kubo in {:?}",
                sender_did.id(),
                refresh_t0.elapsed()
            );
        }

        if message.verify_with_document(&refreshed_document).is_ok() {
            self.set_sender_dirty(&sender_did, false).await;
            return Ok((message, sender_did, refreshed_document));
        }

        self.set_sender_dirty(&sender_did, true).await;
        warn!(
            "DID {} marked dirty: signature verification still failed after forced refresh",
            sender_did.id()
        );

        Err(anyhow!(
            "message signature verification failed for {} (cached and refreshed DID document)",
            sender_did.id()
        ))
    }

    pub(crate) async fn handle_request(&self, request: WorldRequest, agent_endpoint: String) -> Result<WorldResponse> {
        if !self.lane.supports_request(&request) {
            return Err(anyhow!(
                "lane '{}' does not support this request type",
                self.lane.label()
            ));
        }

        // Each ALPN lane has exactly one canonical content type.
        let (message, sender_did, sender_document) = self.verify_message(request.message_cbor).await?;
        let expected_ct = CONTENT_TYPE_WORLD;
        if !Self::content_type_matches(&message.content_type, expected_ct, "application/x-ma-command") {
            return Err(anyhow!("expected {} on this lane, got {}", expected_ct, message.content_type));
        }
        let command: WorldCommand = serde_json::from_slice(&message.content)
            .map_err(|err| anyhow!("invalid command payload: {}", err))?;
        if !self.lane.supports_command(&command) {
            return Err(anyhow!(
                "lane '{}' does not support command on this request type",
                self.lane.label()
            ));
        }
        if let Some(method) = command.internal_method() {
            debug!("processing internal method '{}' on lane '{}'", method, self.lane.label());
        }
        self.handle_command(command, &message, &sender_did, sender_document, agent_endpoint).await
    }

    pub(crate) async fn room_state_response(
        &self,
        room_name: &str,
        message: String,
        latest_event_sequence: u64,
        broadcasted: bool,
        events: Vec<RoomEvent>,
        handle: String,
    ) -> WorldResponse {
        let room_name_owned = room_name.to_string();
        let room_description = self.world.room_description(room_name).await;
        let room_title = self.world.room_title(room_name).await;
        let room_url = self.world.room_url(room_name).await;
        let world_did = self.world.world_did.read().await.clone().unwrap_or_default();
        let avatars = self.world.room_avatars(room_name).await;
        let room_object_dids = self.world.room_object_did_map(room_name).await;

        WorldResponse {
            ok: true,
            room: room_name_owned,
            message,
            endpoint_id: self.endpoint_id.clone(),
            latest_event_sequence,
            broadcasted,
            events,
            handle,
            room_description,
            room_title,
            room_url,
            world_did,
            avatars,
            room_object_dids,
            transport_ack: None,
        }
    }

    pub(crate) async fn handle_command(
        &self,
        command: WorldCommand,
        message: &Message,
        sender_did: &Did,
        sender_document: Document,
        agent_endpoint: String,
    ) -> Result<WorldResponse> {
        let command_kind = match &command {
            WorldCommand::Enter { .. } => "enter",
            WorldCommand::Ping { .. } => "ping",
            WorldCommand::Message { .. } => "message",
            WorldCommand::RoomEvents { .. } => "room_events",
        };
        debug!(
            "request lane='{}' kind='{}' from='{}' to='{}'",
            self.lane.label(),
            command_kind,
            sender_did.id(),
            message.to,
        );

        let sender_profile = sender_profile_from_document(&sender_document);
        let sender_push_endpoint = sender_push_endpoint_from_document(&sender_document)
            .unwrap_or_else(|| agent_endpoint.clone());
        let sender_encryption_pubkey_multibase =
            sender_encryption_pubkey_multibase_from_document(&sender_document)?;
        if sender_push_endpoint != agent_endpoint {
            debug!(
                "sender endpoint drift: request_remote={} did_push_endpoint={} did={}",
                agent_endpoint,
                sender_push_endpoint,
                sender_did.id()
            );
        }

        match command {
            WorldCommand::Enter { room_url } => {
                self.handle_enter(
                    sender_did,
                    &sender_profile,
                    &sender_push_endpoint,
                    &sender_encryption_pubkey_multibase,
                    &agent_endpoint,
                    room_url,
                )
                .await
            }
            WorldCommand::Ping { room_url } => {
                self.handle_ping(sender_did, &sender_push_endpoint, &agent_endpoint, room_url)
                    .await
            }
            WorldCommand::Message { room: _, envelope } => {
                self.handle_message(&message.to, sender_did, &sender_push_endpoint, envelope)
                    .await
            }
            WorldCommand::RoomEvents { room: _, since_sequence } => {
                self.handle_room_events(sender_did, since_sequence).await
            }
        }
    }

}

impl ProtocolHandler for WorldProtocol {
    /// One task runs per connection and serves a single long-lived bi-stream with framed messages.
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let agent_endpoint = connection.remote_id().to_string();
        self.world
            .record_event(format!("connection accepted from {}", agent_endpoint))
            .await;
        let (mut send, mut recv) = connection.accept_bi().await?;

        loop {
            let frame_len = match recv.read_u32().await {
                Ok(n) => n as usize,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(AcceptError::from_err(err)),
            };
            if frame_len > 256 * 1024 {
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("request frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<WorldRequest>(&bytes) {
                Ok(request) => self.process_request(request, agent_endpoint.clone()).await,
                Err(err) => WorldResponse {
                    ok: false,
                    room: String::new(),
                    message: format!("invalid request JSON: {}", err),
                    endpoint_id: self.endpoint_id.clone(),
                    latest_event_sequence: 0,
                    broadcasted: false,
                    events: Vec::new(),
                    handle: String::new(),
                    room_description: String::new(),
                    room_title: String::new(),
                    room_url: String::new(),
                    world_did: String::new(),
                    avatars: Vec::new(),
                    room_object_dids: HashMap::new(),
                    transport_ack: Some(TransportAck {
                        lane: self.lane.label().to_string(),
                        code: TransportAckCode::InvalidRequestJson,
                        detail: format!("invalid request JSON: {}", err),
                    }),
                },
            };
            let payload = serde_json::to_vec(&response).map_err(AcceptError::from_err)?;

            send.write_u32(payload.len() as u32)
                .await
                .map_err(AcceptError::from_err)?;
            send.write_all(&payload).await.map_err(AcceptError::from_err)?;
            send.flush().await.map_err(AcceptError::from_err)?;
        }

        let _ = send.finish();
        Ok(())
    }
}

impl IpfsProtocol {
    pub(crate) async fn process_request(&self, request: WorldRequest) -> IpfsPublishDidResponse {
        match self.handle_request(request).await {
            Ok(response) => response,
            Err(err) => IpfsPublishDidResponse {
                ok: false,
                message: err.to_string(),
                did: None,
                key_name: None,
                cid: None,
            },
        }
    }

    pub(crate) async fn handle_request(&self, request: WorldRequest) -> Result<IpfsPublishDidResponse> {
        let validated = validate_ipfs_publish_request(
            &request.message_cbor,
        )?;

        {
            let mut cache = self.did_cache.write().await;
            cache.insert(
                validated.document_did.ipns.clone(),
                CachedDidDocument {
                    document: validated.document.clone(),
                    dirty: false,
                },
            );
        }

        let (key_name, cid) = publish_did_document_to_kubo(
            &self.kubo_url,
            &validated.request.did_document_json,
            &validated.request.ipns_private_key_base64,
            validated.request.desired_fragment.as_deref(),
        )
        .await?;

        Ok(IpfsPublishDidResponse {
            ok: true,
            message: "did document published via ma/ipfs/1".to_string(),
            did: Some(validated.document_did.id()),
            key_name,
            cid,
        })
    }
}

impl ProtocolHandler for IpfsProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;

        loop {
            let frame_len = match recv.read_u32().await {
                Ok(n) => n as usize,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(AcceptError::from_err(err)),
            };

            if frame_len > 1024 * 1024 {
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("ipfs frame too large: {}", frame_len),
                )));
            }

            let mut bytes = vec![0u8; frame_len];
            recv.read_exact(&mut bytes).await.map_err(AcceptError::from_err)?;

            let response = match serde_json::from_slice::<WorldRequest>(&bytes) {
                Ok(request) => self.process_request(request).await,
                Err(err) => IpfsPublishDidResponse {
                    ok: false,
                    message: format!("invalid ipfs request JSON: {}", err),
                    did: None,
                    key_name: None,
                    cid: None,
                },
            };

            let payload = serde_json::to_vec(&response).map_err(AcceptError::from_err)?;
            send.write_u32(payload.len() as u32)
                .await
                .map_err(AcceptError::from_err)?;
            send.write_all(&payload).await.map_err(AcceptError::from_err)?;
            send.flush().await.map_err(AcceptError::from_err)?;
        }

        let _ = send.finish();
        Ok(())
    }
}

pub(crate) fn derive_world_master_key(secret_key: &SecretKey, world_slug: &str) -> [u8; 32] {
    // Deterministic per-world key derived from machine-local iroh identity.
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/master-key/v1");
    hasher.update(world_slug.as_bytes());
    hasher.update(secret_key.to_bytes());
    let digest = hasher.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn derive_world_signing_private_key(world_master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/did-doc-signing-key/v1");
    hasher.update(world_master_key);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn derive_world_encryption_private_key(world_master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ma-world/did-doc-encryption-key/v1");
    hasher.update(world_master_key);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) async fn ensure_kubo_key_id(kubo_url: &str, key_name: &str) -> Result<String> {
    let mut keys = list_kubo_keys(kubo_url).await?;
    if !keys.iter().any(|key| key.name == key_name) {
        generate_kubo_key(kubo_url, key_name).await?;
        keys = list_kubo_keys(kubo_url).await?;
    }

    keys
        .into_iter()
        .find(|key| key.name == key_name)
        .map(|key| key.id)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow!("kubo key '{}' exists but has no usable id", key_name))
}

/// Resolve the world root CID from the ma.world IPLD link in the DID document.
pub(crate) async fn resolve_world_root_cid_from_did(kubo_url: &str, world_did: &str) -> Result<Option<String>> {
    let world = Did::try_from(world_did)
        .map_err(|e| anyhow!("invalid world DID '{}': {}", world_did, e))?;
    let document = kubo::fetch_did_document(kubo_url, &world).await?;
    let Some(ma) = document.ma.as_ref() else {
        return Ok(None);
    };
    // ma.world may be either:
    // - direct IPLD link: {"/":"<cid>"}
    // - tailored projection object: { owner: {...}, public: {"/":"<cid>"}, root: {"/":"<cid>"} }
    let Some(world_val) = ma.world.as_ref() else {
        return Ok(None);
    };
    let Some(obj) = world_val.as_object() else {
        return Ok(None);
    };

    if let Some(cid_str) = obj.get("/").and_then(|v| v.as_str()) {
        let trimmed = cid_str.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }

    if let Some(root_obj) = obj.get("root").and_then(|v| v.as_object()) {
        if let Some(cid_str) = root_obj.get("/").and_then(|v| v.as_str()) {
            let trimmed = cid_str.trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_string()));
            }
        }
    }

    Ok(None)
}

pub(crate) fn set_document_ma_string_field(document: &mut Document, key: &str, value: &str) -> Result<()> {
    let raw = document
        .marshal()
        .map_err(|e| anyhow!("failed to marshal DID document for ma.{} update: {}", key, e))?;
    let mut json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| anyhow!("failed to decode DID document JSON for ma.{} update: {}", key, e))?;
    let root = json
        .as_object_mut()
        .ok_or_else(|| anyhow!("DID document root is not a JSON object"))?;
    let ma = root
        .entry("ma")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let ma_obj = ma
        .as_object_mut()
        .ok_or_else(|| anyhow!("DID document ma field is not a JSON object"))?;
    ma_obj.insert(key.to_string(), serde_json::Value::String(value.to_string()));
    let updated = serde_json::to_string(&json)
        .map_err(|e| anyhow!("failed to encode DID document JSON for ma.{} update: {}", key, e))?;
    *document = Document::unmarshal(&updated)
        .map_err(|e| anyhow!("failed to reparse DID document after ma.{} update: {}", key, e))?;
    Ok(())
}

pub(crate) fn compiled_default_lang_cid() -> Option<String> {
    const RAW: &str = include_str!("../.lang_cid");
    RAW.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
}

