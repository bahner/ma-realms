use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    http::header,
    Json, Router,
    Form, extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::{World, WorldInfo, WorldSnapshot};

#[derive(Clone)]
struct StatusState {
    world: Arc<World>,
    world_info: WorldInfo,
}

pub async fn serve(listener: TcpListener, world: Arc<World>, world_info: WorldInfo) -> Result<()> {
    let state = StatusState { world, world_info };
    let cors = CorsLayer::new().allow_origin(Any);
    let app = Router::new()
        .route("/", get(index))
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
        .with_state(state);

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

async fn index(State(state): State<StatusState>) -> Html<String> {
    let snapshot = state.world.snapshot().await;
    let (ma_link, ma_link_resolved_root_cid, ma_link_error) = state.world.did_ma_pointer_info().await;
    let (last_save_pointer_publish_ok, last_save_pointer_publish_root_cid, last_save_pointer_publish_error) =
        state.world.last_pointer_publish_status().await;
    let ma_runtime_mode = state.world.ma_runtime_mode().await;
    let runtime = RuntimeStatus {
        unlocked: state.world.is_unlocked().await,
        kubo_url: state.world.kubo_url().await,
        owner_did: state.world.owner_did().await,
        world_cid: state.world.world_cid().await,
        state_cid: state.world.state_cid().await,
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
    Html(render_html(&state.world_info, &snapshot, &runtime))
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
        owner_did: state.world.owner_did().await,
        world_cid: state.world.world_cid().await,
        state_cid: state.world.state_cid().await,
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
                                    "required": ["owner_did"],
                                    "properties": {
                                        "owner_did": { "type": "string" }
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
    match state.world.set_owner_did(&form.owner_did).await {
        Ok(owner_did) => match state.world.save_encrypted_state().await {
            Ok((state_cid, root_cid)) => Json(WorldOwnerResponse {
                ok: true,
                message: format!(
                    "world owner set to '{}' and persisted (state_cid={}, root_cid={})",
                    owner_did, state_cid, root_cid
                ),
                owner_did: Some(owner_did),
            }),
            Err(err) => Json(WorldOwnerResponse {
                ok: false,
                message: format!(
                    "world owner set to '{}' in runtime but persist failed: {}",
                    owner_did, err
                ),
                owner_did: Some(owner_did),
            }),
        },
        Err(err) => Json(WorldOwnerResponse {
            ok: false,
            message: format!("world owner update failed: {}", err),
            owner_did: None,
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
    owner_did: Option<String>,
    world_cid: Option<String>,
    state_cid: Option<String>,
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
    owner_did: Option<String>,
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
    owner_did: String,
}

#[derive(serde::Deserialize)]
struct WorldStateLoadForm {
    state_cid: String,
}

#[derive(serde::Deserialize)]
struct WorldRootLoadForm {
    root_cid: String,
}

fn render_html(world: &WorldInfo, snapshot: &WorldSnapshot, runtime: &RuntimeStatus) -> String {
        if runtime.unlocked {
                render_unlocked_html(world, snapshot, runtime)
        } else {
                render_locked_html(world, runtime)
        }
}

fn render_locked_html(world: &WorldInfo, runtime: &RuntimeStatus) -> String {
        format!(
                "<!doctype html>
<html>
<head>
    <meta charset=\"utf-8\">
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
    <title>{title} - Unlock</title>
    <style>
        :root {{ --bg:#f5efe3; --panel:rgba(255,251,244,0.94); --ink:#2b241a; --muted:#6f6250; --line:rgba(61,50,32,0.12); --accent:#1f5f5b; --accent-soft:rgba(31,95,91,0.12); }}
        * {{ box-sizing: border-box; }}
        body {{ margin:0; font-family:'Avenir Next','Segoe UI',sans-serif; background:radial-gradient(circle at top,#fbf7ef 0%,var(--bg) 58%,#e4dccd 100%); color:var(--ink); }}
        main {{ max-width: 1020px; margin: 0 auto; padding: 28px 18px 42px; display:grid; gap:16px; }}
        .card {{ padding:18px; border-radius:18px; border:1px solid var(--line); background:var(--panel); box-shadow:0 10px 28px rgba(50,36,18,0.06); }}
        .badge {{ display:inline-flex; padding:6px 10px; border-radius:999px; background:var(--accent-soft); color:var(--accent); font-size:0.84rem; }}
        h1 {{ margin:10px 0 6px; font-size: clamp(1.8rem, 4.2vw, 3rem); font-family:'Iowan Old Style','Palatino Linotype',serif; }}
        p {{ margin:0; color:var(--muted); }}
        .grid {{ display:grid; gap:12px; grid-template-columns: repeat(auto-fit,minmax(220px,1fr)); margin-top:14px; }}
        .metric {{ padding:12px; border-radius:14px; border:1px solid var(--line); background:rgba(255,255,255,0.75); }}
        .metric span {{ display:block; font-size:0.76rem; color:var(--muted); text-transform:uppercase; letter-spacing:0.08em; margin-bottom:6px; }}
        code {{ font-family:'SFMono-Regular','Consolas',monospace; word-break:break-all; }}
        .form-grid {{ display:grid; gap:12px; grid-template-columns: repeat(auto-fit,minmax(260px,1fr)); align-items:start; }}
        .group {{ display:grid; gap:8px; }}
        .unlock-layout {{ display:grid; gap:14px; grid-template-columns: 1fr; }}
        label {{ font-size:0.86rem; color:var(--muted); text-transform:uppercase; letter-spacing:0.07em; }}
        input, textarea {{ width:100%; border:1px solid var(--line); border-radius:10px; padding:10px; font:inherit; background:#fff; color:var(--ink); }}
        textarea {{ min-height: 320px; resize: vertical; font-family:'SFMono-Regular','Consolas',monospace; font-size:0.9rem; }}
        .row {{ display:flex; gap:8px; flex-wrap:wrap; }}
        button {{ border:1px solid var(--line); background:#fff; border-radius:10px; padding:8px 12px; cursor:pointer; }}
        button.primary {{ border-color:#184b48; background:#1f5f5b; color:#fff; }}
        .note {{ margin-top:8px; font-size:0.9rem; color:var(--muted); }}
        @media (min-width: 900px) {{ .unlock-layout {{ grid-template-columns: 0.78fr 1.22fr; }} }}
        .result {{ margin-top:12px; padding:10px 12px; border-radius:10px; border:1px solid var(--line); background:rgba(255,255,255,0.8); font-size:0.92rem; white-space:pre-wrap; }}
        .ok {{ border-color:rgba(31,95,91,0.35); background:rgba(31,95,91,0.08); }}
        .err {{ border-color:rgba(140,42,42,0.35); background:rgba(140,42,42,0.08); }}
    </style>
</head>
<body>
    <main>
        <section class=\"card\">
    <link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.ico\">
            <div class=\"badge\">world locked</div>
            <h1>{title}</h1>
            <p>Unlock this world to access the regular status and world controls.</p>
            <div class=\"grid\">
                <div class=\"metric\"><span>World Slug</span><code id=\"slug-metric\">{world_root_pin_name}</code></div>
                <div class=\"metric\"><span>World DID</span><code id=\"world-did-metric\">{world_did}</code></div>
                <div class=\"metric\"><span>Runtime Lock</span><code>locked</code></div>
                <div class=\"metric\"><span>Kubo API</span><code>{kubo_url}</code></div>
                <div class=\"metric\"><span>Iroh Endpoint ID</span><code>{endpoint_id}</code></div>
            </div>
        </section>

        <section class=\"card\">
            <h2 style=\"margin:0 0 10px; font-family:'Iowan Old Style','Palatino Linotype',serif;\">Unlock</h2>
            <div class=\"unlock-layout\">
                <div class=\"group\">
                    <form class=\"group api-form\" method=\"post\" action=\"/world/slug\">
                        <label for=\"slug-input\">Slug</label>
                        <input id=\"slug-input\" name=\"slug\" type=\"text\" value=\"{world_root_pin_name}\" required />
                        <div class=\"row\"><button type=\"submit\">Update slug</button></div>
                    </form>

                    <form class=\"group api-form\" method=\"post\" action=\"/bundle/create\">
                        <label for=\"bundle-passphrase\">Passphrase</label>
                        <input id=\"bundle-passphrase\" name=\"passphrase\" type=\"password\" required />
                        <div class=\"row\"><button type=\"submit\">Create bundle</button></div>
                    </form>
                    <p class=\"note\">Create and keep a fresh bundle in a safe place before unlocking on new machines.</p>
                </div>

                <form class=\"group api-form\" method=\"post\" action=\"/unlock\">
                    <label for=\"unlock-passphrase\">Unlock passphrase</label>
                    <input id=\"unlock-passphrase\" name=\"passphrase\" type=\"password\" required />
                    <label for=\"bundle-input\">Bundle JSON</label>
                    <textarea id=\"bundle-input\" name=\"bundle\" placeholder=\"Paste bundle JSON here\" required></textarea>
                    <div class=\"row\">
                        <button class=\"primary\" type=\"submit\">Unlock world</button>
                        <button type=\"button\" id=\"copy-bundle\">Copy bundle</button>
                        <button type=\"button\" id=\"reset-local-cache\">Reset local cache</button>
                    </div>
                    <p class=\"note\">Unlock reloads this page and opens full world status and controls.</p>
                </form>
            </div>

            <div id=\"result\" class=\"result\" hidden></div>
        </section>
    </main>

    <script>
        const resultEl = document.getElementById('result');
        const slugInput = document.getElementById('slug-input');
        const slugMetric = document.getElementById('slug-metric');
        const bundleInput = document.getElementById('bundle-input');
        const bundlePassphrase = document.getElementById('bundle-passphrase');
        const unlockPassphrase = document.getElementById('unlock-passphrase');
        const copyBundle = document.getElementById('copy-bundle');
        const resetLocalCache = document.getElementById('reset-local-cache');

        function showResult(ok, text) {{
            resultEl.hidden = false;
            resultEl.classList.remove('ok', 'err');
            resultEl.classList.add(ok ? 'ok' : 'err');
            resultEl.textContent = text;
        }}

        const savedSlug = localStorage.getItem('ma.status.slug');
        const savedBundle = localStorage.getItem('ma.status.bundle');
        if (savedSlug && slugInput) {{ slugInput.value = savedSlug; }}
        if (savedBundle && bundleInput) bundleInput.value = savedBundle;

        if (slugInput) {{
            slugInput.addEventListener('input', () => {{
                localStorage.setItem('ma.status.slug', slugInput.value || '');
                slugMetric.textContent = slugInput.value || '{world_root_pin_name}';
            }});
        }}

        if (bundleInput) {{
            bundleInput.addEventListener('input', () => {{
                localStorage.setItem('ma.status.bundle', bundleInput.value || '');
            }});
        }}

        if (copyBundle) {{
            copyBundle.addEventListener('click', async () => {{
                const text = bundleInput.value.trim();
                if (!text) {{ showResult(false, 'nothing to copy: bundle is empty'); return; }}
                try {{
                    await navigator.clipboard.writeText(text);
                    showResult(true, 'bundle copied to clipboard');
                }} catch (error) {{
                    showResult(false, 'copy failed: ' + (error && error.message ? error.message : String(error)));
                }}
            }});
        }}

        if (resetLocalCache) {{
            resetLocalCache.addEventListener('click', () => {{
                localStorage.removeItem('ma.status.slug');
                localStorage.removeItem('ma.status.bundle');
                if (slugInput) slugInput.value = '{world_root_pin_name}';
                if (bundleInput) bundleInput.value = '';
                if (bundlePassphrase) bundlePassphrase.value = '';
                if (unlockPassphrase) unlockPassphrase.value = '';
                if (slugMetric) slugMetric.textContent = '{world_root_pin_name}';
                showResult(true, 'local cache reset (slug + bundle)');
            }});
        }}

        document.querySelectorAll('form.api-form').forEach((form) => {{
            form.addEventListener('submit', async (event) => {{
                event.preventDefault();
                const submitButton = form.querySelector('button[type=\"submit\"]');
                if (submitButton) submitButton.disabled = true;
                try {{
                    const response = await fetch(form.action, {{
                        method: 'POST',
                        headers: {{ 'Content-Type': 'application/x-www-form-urlencoded' }},
                        body: new URLSearchParams(new FormData(form)).toString(),
                    }});
                    const data = await response.json();
                    const ok = Boolean(data.ok);
                    if (ok && data.slug && slugInput) {{
                        slugInput.value = data.slug;
                        slugMetric.textContent = data.slug;
                        localStorage.setItem('ma.status.slug', data.slug);
                    }}
                    if (ok && form.action.endsWith('/bundle/create') && data.bundle) {{
                        bundleInput.value = data.bundle;
                        localStorage.setItem('ma.status.bundle', data.bundle);
                        if (unlockPassphrase && !unlockPassphrase.value && bundlePassphrase && bundlePassphrase.value) {{
                            unlockPassphrase.value = bundlePassphrase.value;
                        }}
                    }}
                    showResult(ok, data.message || JSON.stringify(data, null, 2));
                    if (ok && form.action.endsWith('/unlock')) {{
                        setTimeout(() => window.location.reload(), 250);
                    }}
                }} catch (error) {{
                    showResult(false, 'request failed: ' + (error && error.message ? error.message : String(error)));
                }} finally {{
                    if (submitButton) submitButton.disabled = false;
                }}
            }});
        }});
    </script>
</body>
</html>",
                title = escape_html(&world.name),
                world_root_pin_name = escape_html(&runtime.world_root_pin_name),
        world_did = escape_html(&world.world_did),
                endpoint_id = escape_html(&world.endpoint_id),
                kubo_url = escape_html(&runtime.kubo_url),
        )
}

fn render_unlocked_html(world: &WorldInfo, snapshot: &WorldSnapshot, runtime: &RuntimeStatus) -> String {
        let direct_addrs = render_list(&world.direct_addresses, "No direct addresses published yet.");
        let multiaddrs = render_list(&world.multiaddrs, "No multiaddrs derived yet.");
        let relay_urls = render_list(&world.relay_urls, "No relay URLs available yet.");
        let world_cid_input = runtime.world_cid.as_deref().unwrap_or("");
        let state_cid_input = runtime.state_cid.as_deref().unwrap_or("");
        let actor_count = snapshot.rooms.iter().map(|room| room.avatars.len()).sum::<usize>();

        format!(
                "<!doctype html>
<html>
<head>
    <meta charset=\"utf-8\">
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
    <title>{title} - Status</title>
    <style>
        :root {{ --bg:#f5efe3; --panel:rgba(255,251,244,0.94); --ink:#2b241a; --muted:#6f6250; --line:rgba(61,50,32,0.12); --accent:#1f5f5b; }}
        * {{ box-sizing: border-box; }}
        body {{ margin:0; font-family:'Avenir Next','Segoe UI',sans-serif; background:radial-gradient(circle at top,#fbf7ef 0%,var(--bg) 58%,#e4dccd 100%); color:var(--ink); }}
        main {{ max-width:1120px; margin:0 auto; padding:28px 18px 48px; display:grid; gap:16px; }}
        .card {{ padding:18px; border-radius:18px; border:1px solid var(--line); background:var(--panel); box-shadow:0 10px 28px rgba(50,36,18,0.06); }}
        .badge {{ display:inline-flex; padding:6px 10px; border-radius:999px; background:rgba(31,95,91,0.1); color:var(--accent); font-size:0.84rem; }}
        h1 {{ margin:10px 0 6px; font-size: clamp(1.8rem, 4.2vw, 3rem); font-family:'Iowan Old Style','Palatino Linotype',serif; }}
        p {{ margin:0; color:var(--muted); }}
        .metrics {{ display:grid; gap:12px; grid-template-columns: repeat(auto-fit,minmax(210px,1fr)); margin-top:14px; }}
        .metric {{ padding:12px; border-radius:14px; border:1px solid var(--line); background:rgba(255,255,255,0.75); }}
        .metric span {{ display:block; font-size:0.76rem; color:var(--muted); text-transform:uppercase; letter-spacing:0.08em; margin-bottom:6px; }}
        code {{ font-family:'SFMono-Regular','Consolas',monospace; word-break:break-all; }}
        .controls {{ display:grid; gap:12px; grid-template-columns: repeat(auto-fit,minmax(260px,1fr)); }}
        .control {{ padding:14px; border-radius:16px; border:1px solid var(--line); background:rgba(255,255,255,0.75); }}
        .control h3 {{ margin:0 0 6px; font-size:0.82rem; text-transform:uppercase; letter-spacing:0.08em; color:var(--muted); font-family:'Avenir Next','Segoe UI',sans-serif; }}
        form {{ display:grid; gap:8px; }}
        input {{ width:100%; border:1px solid var(--line); border-radius:10px; padding:9px; font:inherit; background:#fff; color:var(--ink); }}
        button {{ border:1px solid var(--line); background:#fff; border-radius:10px; padding:8px 12px; cursor:pointer; }}
        .result {{ margin-top:12px; padding:10px 12px; border-radius:10px; border:1px solid var(--line); background:rgba(255,255,255,0.8); font-size:0.92rem; white-space:pre-wrap; }}
        .ok {{ border-color:rgba(31,95,91,0.35); background:rgba(31,95,91,0.08); }}
        .err {{ border-color:rgba(140,42,42,0.35); background:rgba(140,42,42,0.08); }}
        .lists {{ display:grid; gap:12px; grid-template-columns: repeat(auto-fit,minmax(260px,1fr)); }}
        ul {{ list-style:none; padding:0; margin:0; display:grid; gap:8px; }}
        li {{ padding:10px 12px; border-radius:12px; border:1px solid rgba(61,50,32,0.08); background:rgba(255,255,255,0.72); }}
        .screen-lock {{ position:fixed; inset:0; z-index:1200; background:radial-gradient(circle at 15% 18%, #ffe9b4 0%, #ffc9a5 23%, #ff96bc 44%, #9f89ff 70%, #4b6bd6 100%); display:grid; place-items:center; padding:20px; animation: lockHue 16s linear infinite; }}
        .screen-lock[hidden] {{ display:none; }}
        .screen-lock-card {{ width:min(980px,100%); border-radius:24px; border:1px solid rgba(250,250,255,0.35); background:linear-gradient(165deg, rgba(255,255,255,0.24) 0%, rgba(255,245,255,0.1) 60%, rgba(233,255,255,0.14) 100%); backdrop-filter: blur(7px) saturate(1.3); box-shadow:0 34px 70px rgba(29,12,68,0.33); padding:16px; display:grid; gap:10px; cursor:pointer; transform:rotate(-0.4deg); }}
        .screen-lock-title {{ margin:0; color:#fff8e7; text-shadow:0 3px 18px rgba(54,20,88,0.45); font-family:'Iowan Old Style','Palatino Linotype',serif; font-size: clamp(1.6rem, 3.8vw, 2.8rem); letter-spacing:0.01em; }}
        .screen-lock-sub {{ margin:0; color:rgba(255,249,234,0.92); font-size:1rem; }}
        #screen-lock-canvas {{ width:100%; min-height:280px; height:min(56vh,460px); border-radius:16px; border:1px dashed rgba(255,255,255,0.36); background:linear-gradient(160deg, rgba(244,252,255,0.12) 0%, rgba(255,231,252,0.1) 45%, rgba(226,243,255,0.18) 100%); }}
        .screen-lock-tip {{ margin:0; color:rgba(255,252,236,0.95); font-size:0.92rem; text-align:right; letter-spacing:0.04em; text-transform:uppercase; }}
        .lock-countdown {{ position:fixed; top:10px; right:12px; z-index:1250; padding:6px 10px; border-radius:999px; border:1px solid rgba(255,255,255,0.45); background:rgba(27,17,56,0.62); color:#fff6d9; font:600 12px/1.2 'SFMono-Regular','Consolas',monospace; letter-spacing:0.06em; text-transform:uppercase; backdrop-filter: blur(4px); }}
        @keyframes lockHue {{
            0% {{ filter:hue-rotate(0deg) saturate(1.08); }}
            50% {{ filter:hue-rotate(26deg) saturate(1.26); }}
            100% {{ filter:hue-rotate(0deg) saturate(1.08); }}
        }}
    </style>
</head>
<body>
    <div id=\"lock-countdown\" class=\"lock-countdown\">LOCK 01:00</div>
    <section id=\"screen-lock-overlay\" class=\"screen-lock\" hidden>
    <link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.ico\">
        <article class=\"screen-lock-card\" id=\"screen-lock-card\">
            <h2 class=\"screen-lock-title\">Pan-Galactic Pause</h2>
            <p class=\"screen-lock-sub\">Klikk en gang for å lande tilbake i status.</p>
            <canvas id=\"screen-lock-canvas\" width=\"1400\" height=\"760\"></canvas>
            <p class=\"screen-lock-tip\">One click to continue</p>
        </article>
    </section>

    <main>
        <section class=\"card\">
            <div class=\"badge\">world status</div>
            <h1>{title}</h1>
            <p>Operational dashboard for world metadata and persistence actions.</p>
            <div class=\"metrics\">
                <div class=\"metric\"><span>World Pin Alias</span><code id=\"world-alias-metric\" data-cid=\"{world_cid}\">{world_root_pin_name} -&gt; {world_cid}</code></div>
                <div class=\"metric\"><span>World DID</span><code id=\"world-did-metric\">{world_did}</code></div>
                <div class=\"metric\"><span>World Owner DID</span><code id=\"owner-did-metric\">{owner_did}</code></div>
                <div class=\"metric\"><span>Encrypted State CID</span><code id=\"state-cid-metric\">{state_cid}</code></div>
                <div class=\"metric\"><span>Kubo API</span><code id=\"kubo-url-metric\">{kubo_url}</code></div>
                <div class=\"metric\"><span>Runtime</span><code>unlocked</code></div>
                <div class=\"metric\"><span>Rooms (live)</span><code>{room_count}</code></div>
                <div class=\"metric\"><span>Avatars (live)</span><code>{actor_count}</code></div>
            </div>
        </section>

        <section class=\"card\">
            <div class=\"controls\">
                <section class=\"control\"><h3>Slug</h3><form method=\"post\" action=\"/world/slug\" class=\"api-form\"><input id=\"slug-input\" name=\"slug\" value=\"{world_root_pin_name}\" required /><button type=\"submit\">Update slug</button></form></section>
                <section class=\"control\"><h3>Kubo API</h3><form method=\"post\" action=\"/world/kubo\" class=\"api-form\"><input id=\"kubo-url-input\" name=\"kubo_url\" value=\"{kubo_url}\" required /><button type=\"submit\">Update Kubo API</button></form></section>
                <section class=\"control\"><h3>World owner</h3><form method=\"post\" action=\"/world/owner\" class=\"api-form\"><input id=\"owner-did-input\" name=\"owner_did\" value=\"{owner_did_input}\" placeholder=\"did:ma:...\" required /><button type=\"submit\">Set owner</button></form></section>
                <section class=\"control\"><h3>Save world</h3><form method=\"post\" action=\"/world/save\" class=\"api-form\"><button type=\"submit\">Save world</button></form></section>
                <section class=\"control\"><h3>Load by State CID</h3><form method=\"post\" action=\"/world/load\" class=\"api-form\"><input id=\"state-cid-input\" name=\"state_cid\" value=\"{state_cid_input}\" placeholder=\"bafy...\" required /><button type=\"submit\">Load state</button></form></section>
                <section class=\"control\"><h3>Load by Root CID</h3><form method=\"post\" action=\"/world/load-root\" class=\"api-form\"><input id=\"root-cid-input\" name=\"root_cid\" value=\"{world_cid_input}\" placeholder=\"bafy...\" required /><button type=\"submit\">Load root</button></form></section>
            </div>
            <div id=\"result\" class=\"result\" hidden></div>
        </section>

        <section class=\"card lists\">
            <section><h3 style=\"margin:0 0 8px;\">Direct Addresses</h3><ul>{direct_addrs}</ul></section>
            <section><h3 style=\"margin:0 0 8px;\">Multiaddrs</h3><ul>{multiaddrs}</ul></section>
            <section><h3 style=\"margin:0 0 8px;\">Relay URLs</h3><ul>{relay_urls}</ul></section>
        </section>
    </main>

    <script>
        const resultEl = document.getElementById('result');
        const slugInput = document.getElementById('slug-input');
        const kuboUrlInput = document.getElementById('kubo-url-input');
        const ownerDidInput = document.getElementById('owner-did-input');
        const stateCidInput = document.getElementById('state-cid-input');
        const rootCidInput = document.getElementById('root-cid-input');
        const stateCidMetric = document.getElementById('state-cid-metric');
        const worldAliasMetric = document.getElementById('world-alias-metric');
        const kuboUrlMetric = document.getElementById('kubo-url-metric');
        const ownerDidMetric = document.getElementById('owner-did-metric');
        const lockCountdown = document.getElementById('lock-countdown');
        const lockOverlay = document.getElementById('screen-lock-overlay');
        const lockCanvas = document.getElementById('screen-lock-canvas');

        const LOCK_AFTER_MS = 60 * 1000;
        let screenLockTimer = null;
        let lockDeadline = Date.now() + LOCK_AFTER_MS;
        let lockCountdownTicker = null;
        let lockAnimationHandle = null;
        const lockStars = Array.from({{ length: 120 }}, () => ({{
            x: Math.random(),
            y: Math.random(),
            size: 0.5 + Math.random() * 1.8,
            twinkle: 0.3 + Math.random() * 1.4,
            drift: (Math.random() - 0.5) * 0.06,
        }}));

        function showResult(ok, text) {{
            resultEl.hidden = false;
            resultEl.classList.remove('ok', 'err');
            resultEl.classList.add(ok ? 'ok' : 'err');
            resultEl.textContent = text;
        }}

        function setKuboUrl(value) {{
            const url = value && value.trim() ? value.trim() : '';
            if (kuboUrlMetric) kuboUrlMetric.textContent = url || '(none)';
            if (kuboUrlInput) kuboUrlInput.value = url;
        }}

        function setOwnerDid(value) {{
            const did = value && value.trim() ? value.trim() : '(none)';
            if (ownerDidMetric) ownerDidMetric.textContent = did;
            if (ownerDidInput) ownerDidInput.value = did === '(none)' ? '' : did;
        }}

        function setStateCid(value) {{
            const cid = value && value.trim() ? value.trim() : '(none)';
            if (stateCidMetric) stateCidMetric.textContent = cid;
            if (stateCidInput && cid !== '(none)') stateCidInput.value = cid;
        }}

        function setRootCid(value) {{
            const cid = value && value.trim() ? value.trim() : '(none)';
            if (worldAliasMetric) {{
                worldAliasMetric.dataset.cid = cid;
                const slug = slugInput && slugInput.value.trim() ? slugInput.value.trim() : '{world_root_pin_name}';
                worldAliasMetric.textContent = slug + ' -> ' + cid;
            }}
            if (rootCidInput && cid !== '(none)') rootCidInput.value = cid;
        }}

        function drawLockCanvas(timeMs) {{
            if (!lockCanvas) return;

            const rect = lockCanvas.getBoundingClientRect();
            const dpr = Math.max(1, window.devicePixelRatio || 1);
            const width = Math.max(320, Math.floor(rect.width));
            const height = Math.max(220, Math.floor(rect.height));
            const t = (timeMs || 0) / 1000;

            lockCanvas.width = Math.floor(width * dpr);
            lockCanvas.height = Math.floor(height * dpr);

            const ctx = lockCanvas.getContext('2d');
            if (!ctx) return;

            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            ctx.clearRect(0, 0, width, height);

            const gradient = ctx.createLinearGradient(0, 0, width, height);
            gradient.addColorStop(0, 'rgba(30,16,58,0.88)');
            gradient.addColorStop(0.5, 'rgba(56,34,96,0.72)');
            gradient.addColorStop(1, 'rgba(20,61,110,0.84)');
            ctx.fillStyle = gradient;
            ctx.fillRect(0, 0, width, height);

            for (let i = 0; i < 4; i++) {{
                const orbX = width * (0.15 + i * 0.23) + Math.sin(t * (0.35 + i * 0.1)) * 26;
                const orbY = height * (0.22 + ((i % 2) * 0.3)) + Math.cos(t * (0.42 + i * 0.08)) * 22;
                const orbGrad = ctx.createRadialGradient(orbX, orbY, 4, orbX, orbY, width * 0.2);
                orbGrad.addColorStop(0, 'rgba(255,181,221,0.24)');
                orbGrad.addColorStop(1, 'rgba(255,181,221,0)');
                ctx.fillStyle = orbGrad;
                ctx.fillRect(0, 0, width, height);
            }}

            for (const star of lockStars) {{
                const sx = (star.x * width + t * 18 * star.drift + width) % width;
                const sy = star.y * height;
                const alpha = 0.35 + 0.65 * Math.abs(Math.sin(t * star.twinkle + star.x * 20));
                ctx.fillStyle = 'rgba(255,255,255,' + alpha.toFixed(3) + ')';
                ctx.beginPath();
                ctx.arc(sx, sy, star.size, 0, Math.PI * 2);
                ctx.fill();
            }}

            const text = `Don't panic`;
            const size = Math.max(58, Math.floor(width * 0.12));
            const baseX = Math.floor(width * 0.1);
            const baseY = Math.floor(height * 0.58);

            ctx.textBaseline = 'middle';
            ctx.lineCap = 'round';
            ctx.lineJoin = 'round';
            ctx.font = '700 ' + size + 'px cursive';

            for (let i = 0; i < 7; i++) {{
                const phase = t * (0.9 + i * 0.06);
                const jx = Math.sin(phase + i * 0.8) * (2 + i * 0.7);
                const jy = Math.cos(phase * 1.1 + i * 0.6) * (1.2 + i * 0.6);
                ctx.save();
                ctx.translate(baseX + jx, baseY + jy);
                ctx.rotate(Math.sin(phase * 0.5 + i) * 0.02);
                ctx.strokeStyle = 'rgba(255,255,255,' + (0.08 + i * 0.07) + ')';
                ctx.lineWidth = 2.3 + i * 0.35;
                ctx.strokeText(text, 0, 0);
                ctx.fillStyle = 'rgba(' + (125 + i * 16) + ',' + (152 + i * 11) + ',' + (255 - i * 13) + ',' + (0.12 + i * 0.08) + ')';
                ctx.fillText(text, 0, 0);
                ctx.restore();
            }}

            ctx.strokeStyle = 'rgba(255,220,164,0.78)';
            ctx.lineWidth = 3.2;
            ctx.beginPath();
            for (let x = 0; x <= width * 0.82; x += 14) {{
                const wobble = Math.sin((x / 31) + t * 2.2) * 4 + Math.cos((x / 47) + t * 1.5) * 2;
                const y = baseY + size * 0.38 + wobble;
                if (x === 0) ctx.moveTo(baseX + x, y);
                else ctx.lineTo(baseX + x, y);
            }}
            ctx.stroke();

            const fishX = Math.floor(width * 0.83 + Math.sin(t * 1.4) * 10);
            const fishY = Math.floor(height * 0.24 + Math.cos(t * 1.7) * 8);
            ctx.strokeStyle = 'rgba(167,255,246,0.9)';
            ctx.lineWidth = 3;
            ctx.beginPath();
            ctx.moveTo(fishX - 30, fishY);
            ctx.quadraticCurveTo(fishX, fishY - 18, fishX + 30, fishY);
            ctx.quadraticCurveTo(fishX, fishY + 18, fishX - 30, fishY);
            ctx.stroke();
            ctx.beginPath();
            ctx.moveTo(fishX + 30, fishY);
            ctx.lineTo(fishX + 44, fishY - 11);
            ctx.lineTo(fishX + 44, fishY + 11);
            ctx.closePath();
            ctx.stroke();
            ctx.fillStyle = 'rgba(167,255,246,0.35)';
            ctx.fill();
            ctx.fillStyle = 'rgba(238,255,255,0.85)';
            ctx.beginPath();
            ctx.arc(fishX - 12, fishY - 3, 2.2, 0, Math.PI * 2);
            ctx.fill();
        }}

        function startLockAnimation() {{
            if (lockAnimationHandle) return;
            const frame = (ts) => {{
                if (!lockOverlay || lockOverlay.hidden) {{
                    lockAnimationHandle = null;
                    return;
                }}
                drawLockCanvas(ts);
                lockAnimationHandle = requestAnimationFrame(frame);
            }};
            lockAnimationHandle = requestAnimationFrame(frame);
        }}

        function stopLockAnimation() {{
            if (!lockAnimationHandle) return;
            cancelAnimationFrame(lockAnimationHandle);
            lockAnimationHandle = null;
        }}

        function formatRemaining(ms) {{
            const total = Math.max(0, Math.ceil(ms / 1000));
            const minutes = Math.floor(total / 60).toString().padStart(2, '0');
            const seconds = (total % 60).toString().padStart(2, '0');
            return minutes + ':' + seconds;
        }}

        function updateLockCountdown() {{
            if (!lockCountdown) return;
            if (lockOverlay && !lockOverlay.hidden) {{
                lockCountdown.textContent = 'LOCKED';
                return;
            }}
            const remainingMs = lockDeadline - Date.now();
            lockCountdown.textContent = 'LOCK ' + formatRemaining(remainingMs);

            // Fallback lock path even if timeout is delayed/throttled.
            if (remainingMs <= 0) {{
                showScreenLock();
            }}
        }}

        function showScreenLock() {{
            if (!lockOverlay || !lockOverlay.hidden) return;
            if (screenLockTimer) {{
                clearTimeout(screenLockTimer);
                screenLockTimer = null;
            }}
            lockOverlay.hidden = false;
            document.body.style.overflow = 'hidden';
            startLockAnimation();
            updateLockCountdown();
        }}

        function hideScreenLock() {{
            if (!lockOverlay || lockOverlay.hidden) return;
            lockOverlay.hidden = true;
            document.body.style.overflow = '';
            stopLockAnimation();
            resetScreenLockTimer();
        }}

        function resetScreenLockTimer() {{
            if (screenLockTimer) clearTimeout(screenLockTimer);
            lockDeadline = Date.now() + LOCK_AFTER_MS;
            screenLockTimer = setTimeout(showScreenLock, LOCK_AFTER_MS);
            updateLockCountdown();
        }}

        if (lockOverlay) {{
            lockOverlay.addEventListener('click', hideScreenLock);
        }}
        if (lockCanvas) {{
            window.addEventListener('resize', () => {{
                if (lockOverlay && !lockOverlay.hidden) drawLockCanvas(performance.now());
            }});
        }}

        ['pointerdown', 'keydown', 'touchstart'].forEach((eventName) => {{
            window.addEventListener(eventName, () => {{
                if (lockOverlay && !lockOverlay.hidden) return;
                resetScreenLockTimer();
            }}, {{ passive: true }});
        }});

        lockCountdownTicker = setInterval(updateLockCountdown, 1000);
        updateLockCountdown();
        resetScreenLockTimer();

        document.querySelectorAll('form.api-form').forEach((form) => {{
            form.addEventListener('submit', async (event) => {{
                event.preventDefault();
                resetScreenLockTimer();
                const submitButton = form.querySelector('button[type=\"submit\"]');
                if (submitButton) submitButton.disabled = true;
                try {{
                    const response = await fetch(form.action, {{
                        method: 'POST',
                        headers: {{ 'Content-Type': 'application/x-www-form-urlencoded' }},
                        body: new URLSearchParams(new FormData(form)).toString(),
                    }});
                    const data = await response.json();
                    const ok = Boolean(data.ok);
                    if (ok && data.slug && slugInput) {{
                        slugInput.value = data.slug;
                        if (worldAliasMetric) {{
                            const cid = worldAliasMetric.dataset.cid || '(none)';
                            worldAliasMetric.textContent = data.slug + ' -> ' + cid;
                        }}
                    }}
                    if (ok && data.kubo_url) setKuboUrl(data.kubo_url);
                    if (ok && data.owner_did) setOwnerDid(data.owner_did);
                    if (ok && data.state_cid) setStateCid(data.state_cid);
                    if (ok && data.root_cid) setRootCid(data.root_cid);
                    if (ok && form.action.endsWith('/world/load') && stateCidInput) setStateCid(stateCidInput.value);
                    showResult(ok, data.message || JSON.stringify(data, null, 2));
                }} catch (error) {{
                    showResult(false, 'request failed: ' + (error && error.message ? error.message : String(error)));
                }} finally {{
                    if (submitButton) submitButton.disabled = false;
                }}
            }});
        }});
    </script>
</body>
</html>",
                title = escape_html(&world.name),
                world_root_pin_name = escape_html(&runtime.world_root_pin_name),
                world_did = escape_html(&world.world_did),
                owner_did = escape_html(runtime.owner_did.as_deref().unwrap_or("(none)")),
                owner_did_input = escape_html(runtime.owner_did.as_deref().unwrap_or("")),
                world_cid = escape_html(runtime.world_cid.as_deref().unwrap_or("(none)")),
                state_cid = escape_html(runtime.state_cid.as_deref().unwrap_or("(none)")),
                state_cid_input = escape_html(state_cid_input),
                world_cid_input = escape_html(world_cid_input),
                kubo_url = escape_html(&runtime.kubo_url),
                room_count = snapshot.rooms.len(),
                actor_count = actor_count,
                direct_addrs = direct_addrs,
                multiaddrs = multiaddrs,
                relay_urls = relay_urls,
        )
}

fn render_list(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        return format!("<li class=\"empty\">{}</li>", escape_html(empty));
    }

    items
        .iter()
        .map(|item| format!("<li><code>{}</code></li>", escape_html(item)))
        .collect::<Vec<_>>()
        .join("")
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
