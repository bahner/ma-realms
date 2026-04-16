use super::*;

impl WorldProtocol {
    pub(super) async fn handle_enter(
        &self,
        sender_identity: &Did,
        sender_profile: &str,
        sender_push_endpoint: &str,
        sender_encryption_pubkey_multibase: &str,
        agent_endpoint: &str,
        room_url: String,
    ) -> Result<WorldResponse> {
        let pinged_room = if room_url.is_empty() {
            DEFAULT_ROOM.to_string()
        } else if let Ok(did) = Did::try_from(room_url.as_str()) {
            did.fragment.clone().unwrap_or_else(|| DEFAULT_ROOM.to_string())
        } else {
            room_url.clone() // Postel: accept plain room name
        };

        let world_ipns = self
            .world
            .local_world_ipns()
            .await
            .unwrap_or_else(|| "unconfigured".to_string());
        let avatar_fragment = sender_identity
            .fragment
            .clone()
            .unwrap_or_else(|| "avatar".to_string())
            .trim()
            .trim_start_matches('@')
            .to_string();
        let avatar_did_str = create_world_did(&world_ipns, &avatar_fragment);

        let (_avatar_did, handle, created) = self
            .world
            .ensure_avatar(
                sender_identity,
                sender_profile,
                sender_push_endpoint,
                sender_encryption_pubkey_multibase,
                &pinged_room,
            )
            .await?;
        let actual_room = pinged_room.clone();

        info!(
            "[{}] worldentry {} did={} ingress_lane={} remote={} push={}",
            actual_room,
            handle,
            avatar_did_str,
            self.lane.label(),
            agent_endpoint,
            sender_push_endpoint
        );

        if created {
            let _ = self
                .world
                .enqueue_room_dispatch(&actual_room, RoomDispatchTask::PresenceSnapshot)
                .await;
        }
        let room_url = self.world.room_url(&actual_room).await;
        let latest_event_sequence = self
            .world
            .latest_room_event_sequence(&actual_room)
            .await
            .unwrap_or(0);
        Ok(self
            .room_state_response(
                &actual_room,
                format!("pong {}", room_url),
                latest_event_sequence,
                false,
                Vec::new(),
                handle,
            )
            .await)
    }

    pub(super) async fn handle_ping(
        &self,
        sender_identity: &Did,
        sender_push_endpoint: &str,
        agent_endpoint: &str,
        room_url: String,
    ) -> Result<WorldResponse> {
        let pinged_room = if room_url.is_empty() {
            DEFAULT_ROOM.to_string()
        } else if let Ok(did) = Did::try_from(room_url.as_str()) {
            did.fragment.clone().unwrap_or_else(|| DEFAULT_ROOM.to_string())
        } else {
            room_url.clone()
        };
        let avatar = self.world.touch_present_avatar(sender_identity).await?;

        debug!(
            "[{}] ping {} did={} requested_room={} ingress_lane={} remote={} push={}",
            avatar.room_name,
            avatar.handle,
            avatar.url.id(),
            pinged_room,
            self.lane.label(),
            agent_endpoint,
            sender_push_endpoint
        );
        let room_url = self.world.room_url(&avatar.room_name).await;
        let latest_event_sequence = self
            .world
            .latest_room_event_sequence(&avatar.room_name)
            .await
            .unwrap_or(0);
        Ok(self
            .room_state_response(
                &avatar.room_name,
                format!("pong {}", room_url),
                latest_event_sequence,
                false,
                Vec::new(),
                avatar.handle,
            )
            .await)
    }

    pub(super) async fn handle_message(
        &self,
        message_to: &str,
        sender_identity: &Did,
        sender_push_endpoint: &str,
        envelope: MessageEnvelope,
    ) -> Result<WorldResponse> {
        let avatar = self.world.require_present_avatar(sender_identity).await?;

        let route_room = match &envelope {
            MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("avatar") => {
                self.world
                    .avatar_room_for_did(&avatar.url.id())
                    .await
                    .unwrap_or_else(|| avatar.room_name.clone())
            }
            _ => avatar.room_name.clone(),
        };

        let _ = self
            .world
            .touch_avatar_presence_for_did(&route_room, &avatar.url.id())
            .await;
        let effective_sender_profile = self
            .world
            .avatar_language_order_for_did(&route_room, &avatar.url.id())
            .await
            .unwrap_or_else(|| "nb_NO:en_UK".to_string());
        let is_world_admin = matches!(
            &envelope,
            MessageEnvelope::ActorCommand { target, .. } if target.eq_ignore_ascii_case("world")
        );
        if is_world_admin && !self.world.is_world_target_did(message_to).await {
            return Err(anyhow!(
                "@world commands must target this world DID; got to='{}'",
                message_to
            ));
        }

        let requested_world_broadcast = match &envelope {
            MessageEnvelope::ActorCommand {
                target,
                command: ActorCommand::Raw { command },
            } if target.eq_ignore_ascii_case("world") => {
                let raw = command.trim();
                if let Some(rest) = raw.strip_prefix("broadcast") {
                    let text = rest.trim();
                    if text.is_empty() {
                        None
                    } else {
                        Some(text.to_string())
                    }
                } else {
                    None
                }
            }
            _ => None,
        };

        // Route command envelopes using the active room handle bound to sender DID.
        let actor_name = self
            .world
            .avatar_handle_for_did(&route_room, &avatar.url.id())
            .await
            .unwrap_or_else(|| avatar.url.id());
        let (message, broadcasted, effective_room) = self
            .world
            .send_message(
                &route_room,
                &actor_name,
                &avatar.url,
                &effective_sender_profile,
                envelope,
            )
            .await?;
        if effective_room != route_room {
            let _ = self
                .world
                .enqueue_room_dispatch(
                    &effective_room,
                    RoomDispatchTask::PresenceRoomStateTo(sender_push_endpoint.to_string()),
                )
                .await;
            let _ = self
                .world
                .enqueue_room_dispatch(&route_room, RoomDispatchTask::PresenceSnapshot)
                .await;
            let _ = self
                .world
                .enqueue_room_dispatch(&effective_room, RoomDispatchTask::PresenceSnapshot)
                .await;
        }
        if let Some(text) = requested_world_broadcast {
            if message.starts_with("@world broadcast sent") {
                let _ = self
                    .world
                    .enqueue_room_dispatch(&effective_room, RoomDispatchTask::WorldBroadcast(text))
                    .await;
            }
        }
        let latest_event_sequence = self.world.latest_room_event_sequence(&effective_room).await?;
        let _ = broadcasted;
        Ok(self
            .room_state_response(
                &effective_room,
                message,
                latest_event_sequence,
                broadcasted,
                Vec::new(),
                avatar.handle,
            )
            .await)
    }

    pub(super) async fn handle_room_events(
        &self,
        sender_identity: &Did,
        since_sequence: u64,
    ) -> Result<WorldResponse> {
        let avatar = self.world.touch_present_avatar(sender_identity).await?;
        let (events, latest_event_sequence) = self
            .world
            .room_events_since(&avatar.room_name, since_sequence)
            .await?;
        Ok(self
            .room_state_response(
                &avatar.room_name,
                String::new(),
                latest_event_sequence,
                false,
                events,
                String::new(),
            )
            .await)
    }
}
