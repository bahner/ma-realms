(async function () {
    'use strict';

    // --- Fetch status data ---
    let data;
    try {
        const resp = await fetch('/status.json');
        data = await resp.json();
    } catch (err) {
        document.body.textContent = 'Failed to load status: ' + err;
        return;
    }

    const world = data.world;
    const runtime = data.runtime;
    const stats = data.stats;

    if (runtime.unlocked) {
        initUnlocked(world, runtime, stats);
    } else {
        initLocked(world, runtime);
    }

    // --- Locked view ---
    function initLocked(world, runtime) {
        const view = document.getElementById('locked-view');
        view.hidden = false;
        document.title = world.name + ' - Unlock';

        setText('locked-title', world.name);
        setText('locked-slug-metric', runtime.world_root_pin_name);
        setText('locked-world-did-metric', world.world_did);
        setText('locked-kubo-metric', runtime.kubo_url);
        setText('locked-endpoint-metric', world.endpoint_id);

        const slugInput = document.getElementById('locked-slug-input');
        const slugMetric = document.getElementById('locked-slug-metric');
        const bundleInput = document.getElementById('bundle-input');
        const bundlePassphrase = document.getElementById('bundle-passphrase');
        const unlockPassphrase = document.getElementById('unlock-passphrase');
        const resultEl = document.getElementById('locked-result');

        if (slugInput) slugInput.value = runtime.world_root_pin_name;

        // Restore from localStorage
        const savedSlug = localStorage.getItem('ma.status.slug');
        const savedBundle = localStorage.getItem('ma.status.bundle');
        if (savedSlug && slugInput) slugInput.value = savedSlug;
        if (savedBundle && bundleInput) bundleInput.value = savedBundle;

        if (slugInput) {
            slugInput.addEventListener('input', () => {
                localStorage.setItem('ma.status.slug', slugInput.value || '');
                if (slugMetric) slugMetric.textContent = slugInput.value || runtime.world_root_pin_name;
            });
        }

        if (bundleInput) {
            bundleInput.addEventListener('input', () => {
                localStorage.setItem('ma.status.bundle', bundleInput.value || '');
            });
        }

        const copyBundle = document.getElementById('copy-bundle');
        if (copyBundle) {
            copyBundle.addEventListener('click', async () => {
                const text = bundleInput ? bundleInput.value.trim() : '';
                if (!text) { showResult(resultEl, false, 'nothing to copy: bundle is empty'); return; }
                try {
                    await navigator.clipboard.writeText(text);
                    showResult(resultEl, true, 'bundle copied to clipboard');
                } catch (error) {
                    showResult(resultEl, false, 'copy failed: ' + (error && error.message ? error.message : String(error)));
                }
            });
        }

        const resetBtn = document.getElementById('reset-local-cache');
        if (resetBtn) {
            resetBtn.addEventListener('click', () => {
                localStorage.removeItem('ma.status.slug');
                localStorage.removeItem('ma.status.bundle');
                if (slugInput) slugInput.value = runtime.world_root_pin_name;
                if (bundleInput) bundleInput.value = '';
                if (bundlePassphrase) bundlePassphrase.value = '';
                if (unlockPassphrase) unlockPassphrase.value = '';
                if (slugMetric) slugMetric.textContent = runtime.world_root_pin_name;
                showResult(resultEl, true, 'local cache reset (slug + bundle)');
            });
        }

        bindForms(view, resultEl, function (form, _data, ok) {
            if (ok && _data.slug && slugInput) {
                slugInput.value = _data.slug;
                if (slugMetric) slugMetric.textContent = _data.slug;
                localStorage.setItem('ma.status.slug', _data.slug);
            }
            if (ok && form.action.endsWith('/bundle/create') && _data.bundle) {
                if (bundleInput) {
                    bundleInput.value = _data.bundle;
                    localStorage.setItem('ma.status.bundle', _data.bundle);
                }
                if (unlockPassphrase && !unlockPassphrase.value && bundlePassphrase && bundlePassphrase.value) {
                    unlockPassphrase.value = bundlePassphrase.value;
                }
            }
            if (ok && form.action.endsWith('/unlock')) {
                setTimeout(() => window.location.reload(), 250);
            }
        });
    }

    // --- Unlocked view ---
    function initUnlocked(world, runtime, stats) {
        const view = document.getElementById('unlocked-view');
        view.hidden = false;
        document.title = world.name + ' - Status';
        document.getElementById('lock-countdown').hidden = false;

        setText('unlocked-title', world.name);
        setText('endpoint-metric', world.endpoint_id);
        setText('actor-web-version-metric',
            (world.actor_web && world.actor_web.version) || '(none)');
        setText('state-cid-metric', runtime.state_cid || '(none)');
        setText('lang-cid-metric', runtime.lang_cid || '(none)');
        setText('room-count-metric', String(stats.room_count));
        setText('avatar-count-metric', String(stats.avatar_count));
        setText('world-did-metric', world.world_did);

        const worldAlias = document.getElementById('world-alias-metric');
        const worldCid = runtime.world_cid || '(none)';
        if (worldAlias) {
            worldAlias.dataset.cid = worldCid;
            worldAlias.textContent = runtime.world_root_pin_name + ' -> ' + worldCid;
        }

        // Populate form inputs
        setVal('slug-input', runtime.world_root_pin_name);
        setVal('kubo-url-input', runtime.kubo_url);
        setVal('owner-did-input', runtime.owner_did || '');
        setVal('state-cid-input', runtime.state_cid || '');
        setVal('root-cid-input', runtime.world_cid || '');

        // Populate address lists
        renderList('direct-addrs-list', world.direct_addresses, 'No direct addresses published yet.');
        renderList('multiaddrs-list', world.multiaddrs, 'No multiaddrs derived yet.');
        renderList('relay-urls-list', world.relay_urls, 'No relay URLs available yet.');

        const resultEl = document.getElementById('unlocked-result');
        const slugInput = document.getElementById('slug-input');

        bindForms(view, resultEl, function (_form, _data, ok) {
            if (ok && _data.slug && slugInput) {
                slugInput.value = _data.slug;
                if (worldAlias) {
                    const cid = worldAlias.dataset.cid || '(none)';
                    worldAlias.textContent = _data.slug + ' -> ' + cid;
                }
            }
            if (ok && _data.kubo_url) setKuboUrl(_data.kubo_url);
            if (ok && _data.owner_did) setOwnerDid(_data.owner_did);
            if (ok && _data.state_cid) setStateCid(_data.state_cid);
            if (ok && _data.root_cid) setRootCid(_data.root_cid);
        });

        initScreenLock();
    }

    // --- Helpers ---
    function setText(id, value) {
        const el = document.getElementById(id);
        if (el) el.textContent = value;
    }

    function setVal(id, value) {
        const el = document.getElementById(id);
        if (el) el.value = value;
    }

    function showResult(el, ok, text) {
        if (!el) return;
        el.hidden = false;
        el.classList.remove('ok', 'err');
        el.classList.add(ok ? 'ok' : 'err');
        el.textContent = text;
    }

    function renderList(id, items, emptyText) {
        const ul = document.getElementById(id);
        if (!ul) return;
        if (!items || items.length === 0) {
            const li = document.createElement('li');
            li.className = 'empty';
            li.textContent = emptyText;
            ul.appendChild(li);
            return;
        }
        for (const item of items) {
            const li = document.createElement('li');
            const code = document.createElement('code');
            code.textContent = item;
            li.appendChild(code);
            ul.appendChild(li);
        }
    }

    function setKuboUrl(value) {
        setVal('kubo-url-input', value || '');
    }

    function setOwnerDid(value) {
        setVal('owner-did-input', value || '');
    }

    function setStateCid(value) {
        const cid = (value && value.trim()) || '(none)';
        setText('state-cid-metric', cid);
        if (cid !== '(none)') setVal('state-cid-input', cid);
    }

    function setRootCid(value) {
        const cid = (value && value.trim()) || '(none)';
        const worldAlias = document.getElementById('world-alias-metric');
        if (worldAlias) {
            worldAlias.dataset.cid = cid;
            const slugInput = document.getElementById('slug-input');
            const slug = (slugInput && slugInput.value.trim()) || '?';
            worldAlias.textContent = slug + ' -> ' + cid;
        }
        if (cid !== '(none)') setVal('root-cid-input', cid);
    }

    function bindForms(container, resultEl, onSuccess) {
        container.querySelectorAll('form.api-form').forEach(function (form) {
            form.addEventListener('submit', async function (event) {
                event.preventDefault();
                const btn = form.querySelector('button[type="submit"]');
                if (btn) btn.disabled = true;
                try {
                    const resp = await fetch(form.action, {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
                        body: new URLSearchParams(new FormData(form)).toString(),
                    });
                    const json = await resp.json();
                    const ok = Boolean(json.ok);
                    showResult(resultEl, ok, json.message || JSON.stringify(json, null, 2));
                    if (onSuccess) onSuccess(form, json, ok);
                } catch (error) {
                    showResult(resultEl, false, 'request failed: ' + (error && error.message ? error.message : String(error)));
                } finally {
                    if (btn) btn.disabled = false;
                }
            });
        });
    }

    // --- Screen lock (unlocked page only) ---
    function initScreenLock() {
        const LOCK_AFTER_MS = 60 * 1000;
        const lockCountdown = document.getElementById('lock-countdown');
        const lockOverlay = document.getElementById('screen-lock-overlay');
        const lockCanvas = document.getElementById('screen-lock-canvas');

        let screenLockTimer = null;
        let lockDeadline = Date.now() + LOCK_AFTER_MS;
        let lockCountdownTicker = null;
        let lockAnimationHandle = null;

        const lockStars = Array.from({ length: 120 }, () => ({
            x: Math.random(),
            y: Math.random(),
            size: 0.5 + Math.random() * 1.8,
            twinkle: 0.3 + Math.random() * 1.4,
            drift: (Math.random() - 0.5) * 0.06,
        }));

        function drawLockCanvas(timeMs) {
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

            for (let i = 0; i < 4; i++) {
                const orbX = width * (0.15 + i * 0.23) + Math.sin(t * (0.35 + i * 0.1)) * 26;
                const orbY = height * (0.22 + ((i % 2) * 0.3)) + Math.cos(t * (0.42 + i * 0.08)) * 22;
                const orbGrad = ctx.createRadialGradient(orbX, orbY, 4, orbX, orbY, width * 0.2);
                orbGrad.addColorStop(0, 'rgba(255,181,221,0.24)');
                orbGrad.addColorStop(1, 'rgba(255,181,221,0)');
                ctx.fillStyle = orbGrad;
                ctx.fillRect(0, 0, width, height);
            }

            for (const star of lockStars) {
                const sx = (star.x * width + t * 18 * star.drift + width) % width;
                const sy = star.y * height;
                const alpha = 0.35 + 0.65 * Math.abs(Math.sin(t * star.twinkle + star.x * 20));
                ctx.fillStyle = 'rgba(255,255,255,' + alpha.toFixed(3) + ')';
                ctx.beginPath();
                ctx.arc(sx, sy, star.size, 0, Math.PI * 2);
                ctx.fill();
            }

            const text = "Don't panic";
            const size = Math.max(58, Math.floor(width * 0.12));
            const baseX = Math.floor(width * 0.1);
            const baseY = Math.floor(height * 0.58);

            ctx.textBaseline = 'middle';
            ctx.lineCap = 'round';
            ctx.lineJoin = 'round';
            ctx.font = '700 ' + size + 'px cursive';

            for (let i = 0; i < 7; i++) {
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
            }

            ctx.strokeStyle = 'rgba(255,220,164,0.78)';
            ctx.lineWidth = 3.2;
            ctx.beginPath();
            for (let x = 0; x <= width * 0.82; x += 14) {
                const wobble = Math.sin((x / 31) + t * 2.2) * 4 + Math.cos((x / 47) + t * 1.5) * 2;
                const y = baseY + size * 0.38 + wobble;
                if (x === 0) ctx.moveTo(baseX + x, y);
                else ctx.lineTo(baseX + x, y);
            }
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
        }

        function startLockAnimation() {
            if (lockAnimationHandle) return;
            const frame = (ts) => {
                if (!lockOverlay || lockOverlay.hidden) {
                    lockAnimationHandle = null;
                    return;
                }
                drawLockCanvas(ts);
                lockAnimationHandle = requestAnimationFrame(frame);
            };
            lockAnimationHandle = requestAnimationFrame(frame);
        }

        function stopLockAnimation() {
            if (!lockAnimationHandle) return;
            cancelAnimationFrame(lockAnimationHandle);
            lockAnimationHandle = null;
        }

        function formatRemaining(ms) {
            const total = Math.max(0, Math.ceil(ms / 1000));
            const minutes = Math.floor(total / 60).toString().padStart(2, '0');
            const seconds = (total % 60).toString().padStart(2, '0');
            return minutes + ':' + seconds;
        }

        function updateLockCountdown() {
            if (!lockCountdown) return;
            if (lockOverlay && !lockOverlay.hidden) {
                lockCountdown.textContent = 'LOCKED';
                return;
            }
            const remainingMs = lockDeadline - Date.now();
            lockCountdown.textContent = 'LOCK ' + formatRemaining(remainingMs);
            if (remainingMs <= 0) showScreenLock();
        }

        function showScreenLock() {
            if (!lockOverlay || !lockOverlay.hidden) return;
            if (screenLockTimer) { clearTimeout(screenLockTimer); screenLockTimer = null; }
            lockOverlay.hidden = false;
            document.body.style.overflow = 'hidden';
            startLockAnimation();
            updateLockCountdown();
        }

        function hideScreenLock() {
            if (!lockOverlay || lockOverlay.hidden) return;
            lockOverlay.hidden = true;
            document.body.style.overflow = '';
            stopLockAnimation();
            resetScreenLockTimer();
        }

        function resetScreenLockTimer() {
            if (screenLockTimer) clearTimeout(screenLockTimer);
            lockDeadline = Date.now() + LOCK_AFTER_MS;
            screenLockTimer = setTimeout(showScreenLock, LOCK_AFTER_MS);
            updateLockCountdown();
        }

        if (lockOverlay) lockOverlay.addEventListener('click', hideScreenLock);
        if (lockCanvas) {
            window.addEventListener('resize', () => {
                if (lockOverlay && !lockOverlay.hidden) drawLockCanvas(performance.now());
            });
        }

        ['pointerdown', 'keydown', 'touchstart'].forEach((evt) => {
            window.addEventListener(evt, () => {
                if (lockOverlay && !lockOverlay.hidden) return;
                resetScreenLockTimer();
            }, { passive: true });
        });

        lockCountdownTicker = setInterval(updateLockCountdown, 1000);
        updateLockCountdown();
        resetScreenLockTimer();
    }
})();
