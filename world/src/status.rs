use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    http::header,
    Json, Router,
    Form, extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::{World, WorldInfo};

#[derive(Clone)]
struct StatusState {
    world: Arc<World>,
    world_info: WorldInfo,
}

pub async fn serve(listener: TcpListener, world: Arc<World>, world_info: WorldInfo, www_root: PathBuf) -> Result<()> {
    let state = StatusState { world, world_info };
    let cors = CorsLayer::new().allow_origin(Any);
    let app = Router::new()
        .route("/favicon.ico", get(favicon_ico))
        .route("/openapi.json", get(openapi_json))
        .route("/actor/web/info", get(actor_web_info_json))
        .route("/unlock", post(unlock_runtime))
        .route("/bundle/create", post(create_unlock_bundle))
        .route("/world/kubo", post(update_kubo_api))
        .route("/world/owner", post(update_world_owner))
        .route("/world/slug", post(update_world_slug))
        .route("/world/save", post(save_world_state))
        .route("/world/load", post(load_world_state))
        .route("/world/load-root", post(load_world_root_index))
        .route("/status.json", get(status_json))
        .layer(cors)
        .with_state(state)
        .fallback_service(ServeDir::new(www_root));

    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn serve_actor_web(listener: TcpListener, web_root: PathBuf) -> Result<()> {
    let app = Router::new().fallback_service(ServeDir::new(web_root));
    axum::serve(listener, app).await?;
    Ok(())
}

async fn favicon_ico() -> impl IntoResponse {
    const ICON_SVG: &str = r#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>
<rect width='64' height='64' rx='12' fill='#0f3345'/>
<text x='32' y='44' text-anchor='middle' font-family='serif' font-size='42' fill='#f8f0dc'>間</text>
</svg>"#;
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        ICON_SVG,
    )
}

async fn status_json(State(state): State<StatusState>) -> Json<StatusDocument> {
    let snapshot = state.world.snapshot().await;
    let room_count = snapshot.rooms.len();
    let avatar_count = snapshot
        .rooms
        .iter()
        .map(|room| room.avatars.len())
        .sum::<usize>();
    let recent_event_count = snapshot.recent_events.len();
    let (ma_link, ma_link_resolved_root_cid, ma_link_error) = state.world.did_ma_pointer_info().await;
    let (last_save_pointer_publish_ok, last_save_pointer_publish_root_cid, last_save_pointer_publish_error) =
        state.world.last_pointer_publish_status().await;
    let ma_runtime_mode = state.world.ma_runtime_mode().await;
    let runtime = RuntimeStatus {
        unlocked: state.world.is_unlocked().await,
        kubo_url: state.world.kubo_url().await,
        owner: state.world.owner_did().await,
        world_cid: state.world.world_cid().await,
        state_cid: state.world.state_cid().await,
        lang_cid: state.world.lang_cid().await,
        persisted_room_count: state.world.persisted_room_count().await,
        world_root_pin_name: state.world.world_root_pin_name().await,
        ma_runtime_mode,
        ma_link,
        ma_link_resolved_root_cid,
        ma_link_error,
        last_save_pointer_publish_ok,
        last_save_pointer_publish_root_cid,
        last_save_pointer_publish_error,
    };
    Json(StatusDocument {
        world: state.world_info,
        runtime,
        stats: StatusStats {
            room_count,
            avatar_count,
            recent_event_count,
        },
    })
}

async fn actor_web_info_json(State(state): State<StatusState>) -> Json<serde_json::Value> {
    if let Some(actor_web) = state.world_info.actor_web.as_ref() {
        Json(serde_json::json!({
            "enabled": true,
            "version": &actor_web.version,
            "cid": &actor_web.cid,
            "status_url": &actor_web.status_url,
            "source_dir": &actor_web.source_dir,
        }))
    } else {
        Json(serde_json::json!({
            "enabled": false,
        }))
    }
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "ma-world status API",
            "version": "1.0.0"
        },
        "paths": {
            "/status.json": {
                "get": {
                    "summary": "Read world status snapshot"
                }
            },
            "/actor/web/info": {
                "get": {
                    "summary": "Read configured actor web runtime metadata"
                }
            },
            "/bundle/create": {
                "post": {
                    "summary": "Create unlock bundle",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["passphrase"],
                                    "properties": {
                                        "passphrase": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/unlock": {
                "post": {
                    "summary": "Unlock runtime",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["passphrase", "bundle"],
                                    "properties": {
                                        "passphrase": { "type": "string" },
                                        "bundle": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/world/slug": {
                "post": {
                    "summary": "Update world slug / pin alias",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["slug"],
                                    "properties": {
                                        "slug": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/world/kubo": {
                "post": {
                    "summary": "Update runtime Kubo API URL",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["kubo_url"],
                                    "properties": {
                                        "kubo_url": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/world/owner": {
                "post": {
                    "summary": "Set world owner DID",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["owner"],
                                    "properties": {
                                        "owner": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/world/save": {
                "post": {
                    "summary": "Save encrypted runtime state"
                }
            },
            "/world/load": {
                "post": {
                    "summary": "Load encrypted runtime state",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["state_cid"],
                                    "properties": {
                                        "state_cid": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/world/load-root": {
                "post": {
                    "summary": "Load world root index",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/x-www-form-urlencoded": {
                                "schema": {
                                    "type": "object",
                                    "required": ["root_cid"],
                                    "properties": {
                                        "root_cid": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }))
}

async fn unlock_runtime(
    State(state): State<StatusState>,
    Form(form): Form<UnlockForm>,
) -> Json<UnlockResponse> {
    match state.world.unlock_runtime(&form.passphrase, &form.bundle).await {
        Ok(count) => Json(UnlockResponse {
            ok: true,
            message: format!("world unlocked ({} actor secret bundle(s) loaded)", count),
        }),
        Err(err) => Json(UnlockResponse {
            ok: false,
            message: format!("unlock failed: {}", err),
        }),
    }
}

async fn create_unlock_bundle(
    State(state): State<StatusState>,
    Form(form): Form<BundleCreateForm>,
) -> Json<BundleCreateResponse> {
    match state.world.create_unlock_bundle(&form.passphrase).await {
        Ok(bundle) => Json(BundleCreateResponse {
            ok: true,
            message: "bundle created".to_string(),
            bundle: Some(bundle),
        }),
        Err(err) => Json(BundleCreateResponse {
            ok: false,
            message: format!("bundle creation failed: {}", err),
            bundle: None,
        }),
    }
}

async fn update_world_slug(
    State(state): State<StatusState>,
    Form(form): Form<WorldSlugForm>,
) -> Json<WorldSlugResponse> {
    match state.world.set_world_root_pin_name(&form.slug).await {
        Ok(slug) => Json(WorldSlugResponse {
            ok: true,
            message: format!("world slug updated to '{}'", slug),
            slug: Some(slug),
        }),
        Err(err) => Json(WorldSlugResponse {
            ok: false,
            message: format!("world slug update failed: {}", err),
            slug: None,
        }),
    }
}

async fn update_kubo_api(
    State(state): State<StatusState>,
    Form(form): Form<KuboApiForm>,
) -> Json<KuboApiResponse> {
    match state.world.set_kubo_url(&form.kubo_url).await {
        Ok(kubo_url) => Json(KuboApiResponse {
            ok: true,
            message: format!("kubo api updated to '{}'", kubo_url),
            kubo_url: Some(kubo_url),
        }),
        Err(err) => Json(KuboApiResponse {
            ok: false,
            message: format!("kubo api update failed: {}", err),
            kubo_url: None,
        }),
    }
}

async fn update_world_owner(
    State(state): State<StatusState>,
    Form(form): Form<WorldOwnerForm>,
) -> Json<WorldOwnerResponse> {
    match state.world.set_owner_did(&form.owner).await {
        Ok(owner) => match state.world.save_encrypted_state().await {
            Ok((state_cid, root_cid)) => Json(WorldOwnerResponse {
                ok: true,
                message: format!(
                    "world owner set to '{}' and persisted (state_cid={}, root_cid={})",
                    owner, state_cid, root_cid
                ),
                owner: Some(owner),
            }),
            Err(err) => Json(WorldOwnerResponse {
                ok: false,
                message: format!(
                    "world owner set to '{}' in runtime but persist failed: {}",
                    owner, err
                ),
                owner: Some(owner),
            }),
        },
        Err(err) => Json(WorldOwnerResponse {
            ok: false,
            message: format!("world owner update failed: {}", err),
            owner: None,
        }),
    }
}

async fn save_world_state(State(state): State<StatusState>) -> Json<WorldStateSaveResponse> {
    match state.world.save_encrypted_state().await {
        Ok((state_cid, root_cid)) => Json(WorldStateSaveResponse {
            ok: true,
            message: "world state saved".to_string(),
            state_cid: Some(state_cid),
            root_cid: Some(root_cid),
        }),
        Err(err) => Json(WorldStateSaveResponse {
            ok: false,
            message: format!("world save failed: {}", err),
            state_cid: None,
            root_cid: None,
        }),
    }
}

async fn load_world_state(
    State(state): State<StatusState>,
    Form(form): Form<WorldStateLoadForm>,
) -> Json<WorldStateLoadResponse> {
    match state.world.load_encrypted_state(&form.state_cid).await {
        Ok(root_cid) => Json(WorldStateLoadResponse {
            ok: true,
            message: format!("world state loaded from {}", form.state_cid),
            root_cid: Some(root_cid),
            rooms_loaded: None,
        }),
        Err(err) => Json(WorldStateLoadResponse {
            ok: false,
            message: format!("world state load failed: {}", err),
            root_cid: None,
            rooms_loaded: None,
        }),
    }
}

async fn load_world_root_index(
    State(state): State<StatusState>,
    Form(form): Form<WorldRootLoadForm>,
) -> Json<WorldStateLoadResponse> {
    match state.world.load_from_world_cid(&form.root_cid).await {
        Ok(rooms_loaded) => Json(WorldStateLoadResponse {
            ok: true,
            message: format!("world root loaded from {}", form.root_cid),
            root_cid: Some(form.root_cid),
            rooms_loaded: Some(rooms_loaded),
        }),
        Err(err) => Json(WorldStateLoadResponse {
            ok: false,
            message: format!("world root load failed: {}", err),
            root_cid: None,
            rooms_loaded: None,
        }),
    }
}

#[derive(serde::Serialize)]
struct StatusDocument {
    world: WorldInfo,
    runtime: RuntimeStatus,
    stats: StatusStats,
}

#[derive(serde::Serialize)]
struct StatusStats {
    room_count: usize,
    avatar_count: usize,
    recent_event_count: usize,
}

#[derive(serde::Serialize)]
struct RuntimeStatus {
    unlocked: bool,
    kubo_url: String,
    owner: Option<String>,
    world_cid: Option<String>,
    state_cid: Option<String>,
    lang_cid: Option<String>,
    persisted_room_count: usize,
    world_root_pin_name: String,
    ma_runtime_mode: String,
    ma_link: Option<String>,
    ma_link_resolved_root_cid: Option<String>,
    ma_link_error: Option<String>,
    last_save_pointer_publish_ok: Option<bool>,
    last_save_pointer_publish_root_cid: Option<String>,
    last_save_pointer_publish_error: Option<String>,
}

#[derive(serde::Serialize)]
struct UnlockResponse {
    ok: bool,
    message: String,
}

#[derive(serde::Serialize)]
struct BundleCreateResponse {
    ok: bool,
    message: String,
    bundle: Option<String>,
}

#[derive(serde::Serialize)]
struct WorldSlugResponse {
    ok: bool,
    message: String,
    slug: Option<String>,
}

#[derive(serde::Serialize)]
struct KuboApiResponse {
    ok: bool,
    message: String,
    kubo_url: Option<String>,
}

#[derive(serde::Serialize)]
struct WorldOwnerResponse {
    ok: bool,
    message: String,
    owner: Option<String>,
}

#[derive(serde::Serialize)]
struct WorldStateSaveResponse {
    ok: bool,
    message: String,
    state_cid: Option<String>,
    root_cid: Option<String>,
}

#[derive(serde::Serialize)]
struct WorldStateLoadResponse {
    ok: bool,
    message: String,
    root_cid: Option<String>,
    rooms_loaded: Option<usize>,
}

#[derive(serde::Deserialize)]
struct UnlockForm {
    passphrase: String,
    bundle: String,
}

#[derive(serde::Deserialize)]
struct BundleCreateForm {
    passphrase: String,
}

#[derive(serde::Deserialize)]
struct WorldSlugForm {
    slug: String,
}

#[derive(serde::Deserialize)]
struct KuboApiForm {
    kubo_url: String,
}

#[derive(serde::Deserialize)]
struct WorldOwnerForm {
    owner: String,
}

#[derive(serde::Deserialize)]
struct WorldStateLoadForm {
    state_cid: String,
}

#[derive(serde::Deserialize)]
struct WorldRootLoadForm {
    root_cid: String,
}

