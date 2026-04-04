export function createUiFlow({
  byId,
  state,
  setSetupStatus,
  ipfsGatewayFallbacks = [],
  lockOverlayTargetFps = 45,
  lockOverlayMaxDpr = 1.4,
}) {
  function stopLockOverlayAnimation() {
    if (state.lockOverlayAnimationId) {
      cancelAnimationFrame(state.lockOverlayAnimationId);
      state.lockOverlayAnimationId = 0;
    }
  }

  function hideLockOverlay() {
    const overlay = byId('lock-overlay');
    if (!overlay) return;
    overlay.classList.add('hidden');
    overlay.setAttribute('aria-hidden', 'true');
    stopLockOverlayAnimation();
  }

  function drawLockOverlayScene() {
    const canvas = byId('lock-canvas');
    if (!canvas) return;

    const dpr = Math.min(lockOverlayMaxDpr, Math.max(1, window.devicePixelRatio || 1));
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;

    const width = Math.floor(rect.width * dpr);
    const height = Math.floor(rect.height * dpr);
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const fishSchool = [
      { speed: 0.07, depth: 0.08, size: 1.0, dir: 1, colorA: 'rgba(114, 236, 255, 0.95)', colorB: 'rgba(116, 160, 255, 0.86)' },
      { speed: 0.055, depth: 0.18, size: 0.82, dir: -1, colorA: 'rgba(255, 188, 132, 0.95)', colorB: 'rgba(247, 107, 165, 0.82)' },
      { speed: 0.082, depth: 0.25, size: 0.72, dir: 1, colorA: 'rgba(146, 255, 214, 0.95)', colorB: 'rgba(77, 206, 255, 0.84)' },
      { speed: 0.048, depth: 0.33, size: 1.18, dir: -1, colorA: 'rgba(174, 205, 255, 0.94)', colorB: 'rgba(122, 139, 255, 0.86)' }
    ];
    const starCount = 72;
    const minFrameMs = 1000 / lockOverlayTargetFps;
    let lastRenderMs = 0;

    function drawFish(x, y, scale, dir, phase, colorA, colorB) {
      const bodyLen = 46 * scale;
      const bodyH = 18 * scale;
      const tailW = 16 * scale;
      const tailSwing = Math.sin(phase * 8.5) * 4 * scale;

      ctx.save();
      ctx.translate(x, y);
      // Fish body points left by default, so flip by -dir to align heading with travel direction.
      ctx.scale(-dir, 1);

      const fishGrad = ctx.createLinearGradient(-bodyLen * 0.5, 0, bodyLen * 0.4, 0);
      fishGrad.addColorStop(0, colorA);
      fishGrad.addColorStop(1, colorB);

      ctx.fillStyle = fishGrad;
      ctx.beginPath();
      ctx.moveTo(-bodyLen * 0.56, 0);
      ctx.quadraticCurveTo(-bodyLen * 0.1, -bodyH * 0.95, bodyLen * 0.52, 0);
      ctx.quadraticCurveTo(-bodyLen * 0.1, bodyH * 0.95, -bodyLen * 0.56, 0);
      ctx.closePath();
      ctx.fill();

      ctx.fillStyle = 'rgba(232, 252, 255, 0.32)';
      ctx.beginPath();
      ctx.ellipse(-bodyLen * 0.08, -bodyH * 0.2, bodyLen * 0.24, bodyH * 0.26, 0, 0, Math.PI * 2);
      ctx.fill();

      ctx.fillStyle = 'rgba(173, 244, 255, 0.85)';
      ctx.beginPath();
      ctx.moveTo(bodyLen * 0.52, 0);
      ctx.lineTo(bodyLen * 0.52 + tailW, -bodyH * 0.58 + tailSwing);
      ctx.lineTo(bodyLen * 0.52 + tailW, bodyH * 0.58 + tailSwing);
      ctx.closePath();
      ctx.fill();

      ctx.fillStyle = 'rgba(250, 255, 255, 0.92)';
      ctx.beginPath();
      ctx.arc(-bodyLen * 0.3, -bodyH * 0.15, 1.9 * scale, 0, Math.PI * 2);
      ctx.fill();

      ctx.restore();
    }

    const paint = (nowMs = performance.now()) => {
      if (lastRenderMs && nowMs - lastRenderMs < minFrameMs) {
        state.lockOverlayAnimationId = requestAnimationFrame(paint);
        return;
      }
      lastRenderMs = nowMs;

      const t = nowMs * 0.001;
      const w = canvas.width;
      const h = canvas.height;
      const horizon = h * 0.67;

      const bg = ctx.createLinearGradient(0, 0, 0, h);
      bg.addColorStop(0, '#070f22');
      bg.addColorStop(0.45, '#101f48');
      bg.addColorStop(1, '#1a1a42');
      ctx.fillStyle = bg;
      ctx.fillRect(0, 0, w, h);

      const nebulaA = ctx.createRadialGradient(
        w * (0.25 + 0.03 * Math.sin(t * 0.3)),
        h * 0.26,
        12 * dpr,
        w * 0.25,
        h * 0.26,
        w * 0.42
      );
      nebulaA.addColorStop(0, 'rgba(255, 130, 190, 0.27)');
      nebulaA.addColorStop(0.55, 'rgba(132, 112, 255, 0.18)');
      nebulaA.addColorStop(1, 'rgba(42, 60, 120, 0)');
      ctx.fillStyle = nebulaA;
      ctx.fillRect(0, 0, w, h);

      const nebulaB = ctx.createRadialGradient(
        w * 0.77,
        h * (0.32 + 0.02 * Math.sin(t * 0.47)),
        16 * dpr,
        w * 0.77,
        h * 0.32,
        w * 0.46
      );
      nebulaB.addColorStop(0, 'rgba(106, 240, 255, 0.2)');
      nebulaB.addColorStop(0.52, 'rgba(83, 145, 255, 0.15)');
      nebulaB.addColorStop(1, 'rgba(20, 38, 92, 0)');
      ctx.fillStyle = nebulaB;
      ctx.fillRect(0, 0, w, h);

      state.lockOverlayStarDrift += 0.34;
      const drift = state.lockOverlayStarDrift;
      for (let i = 0; i < starCount; i += 1) {
        const sx = ((i * 313 + drift * (i % 9 + 2)) % (w + 160)) - 80;
        const sy = ((i * 181) % Math.max(1, Math.floor(horizon))) + (i % 7 === 0 ? Math.sin(t + i) * 4 * dpr : 0);
        const twinkle = 0.24 + 0.68 * Math.abs(Math.sin(t * (0.5 + (i % 5) * 0.2) + i));
        const radius = (0.6 + (i % 4) * 0.55) * dpr;
        ctx.fillStyle = `rgba(222, 243, 255, ${Math.min(0.95, twinkle)})`;
        ctx.beginPath();
        ctx.arc(sx, sy, radius, 0, Math.PI * 2);
        ctx.fill();
      }

      const cometPhase = (t * 0.08) % 1;
      const cometX = w * (1.1 - cometPhase * 1.2);
      const cometY = h * (0.12 + cometPhase * 0.34);
      const tail = ctx.createLinearGradient(cometX - w * 0.22, cometY - h * 0.08, cometX, cometY);
      tail.addColorStop(0, 'rgba(130, 220, 255, 0)');
      tail.addColorStop(1, 'rgba(230, 250, 255, 0.75)');
      ctx.strokeStyle = tail;
      ctx.lineWidth = 3 * dpr;
      ctx.beginPath();
      ctx.moveTo(cometX - w * 0.2, cometY - h * 0.075);
      ctx.lineTo(cometX, cometY);
      ctx.stroke();

      ctx.fillStyle = 'rgba(240, 252, 255, 0.95)';
      ctx.beginPath();
      ctx.arc(cometX, cometY, 3.2 * dpr, 0, Math.PI * 2);
      ctx.fill();

      const sea = ctx.createLinearGradient(0, horizon, 0, h);
      sea.addColorStop(0, '#0c2d66');
      sea.addColorStop(1, '#0b1232');
      ctx.fillStyle = sea;
      ctx.fillRect(0, horizon, w, h - horizon);

      for (let i = 0; i < 5; i += 1) {
        const y = horizon + (12 + i * 14) * dpr;
        const amp = (3 + i * 1.25) * dpr;
        const alpha = 0.22 + (i / 8);
        ctx.strokeStyle = `rgba(120, 214, 255, ${alpha})`;
        ctx.lineWidth = (1.2 + i * 0.2) * dpr;
        ctx.beginPath();
        for (let x = 0; x <= w; x += 16) {
          const wave = Math.sin((x / w) * Math.PI * (5.4 + i * 0.3) + t * (1.1 + i * 0.15)) * amp;
          if (x === 0) ctx.moveTo(x, y + wave);
          else ctx.lineTo(x, y + wave);
        }
        ctx.stroke();
      }

      for (let i = 0; i < fishSchool.length; i += 1) {
        const fish = fishSchool[i];
        const progress = (t * fish.speed + i * 0.29) % 1;
        const laneY = horizon + fish.depth * (h - horizon);
        const bob = Math.sin(t * (1.4 + i * 0.27) + i * 1.3) * (6 + i * 1.8) * dpr;
        const x = fish.dir > 0
          ? -w * 0.18 + progress * w * 1.4
          : w * 1.18 - progress * w * 1.4;
        const y = laneY + bob;

        drawFish(x, y, fish.size * dpr, fish.dir, t + i * 0.6, fish.colorA, fish.colorB);

        // Soft bubble trail behind each fish to emphasize movement.
        for (let b = 0; b < 4; b += 1) {
          const trail = progress - b * 0.013;
          const bx = fish.dir > 0
            ? -w * 0.18 + trail * w * 1.4 - 18 * dpr
            : w * 1.18 - trail * w * 1.4 + 18 * dpr;
          const by = y - b * 6 * dpr - Math.sin(t * 1.6 + b + i) * 2.5 * dpr;
          const alpha = Math.max(0, 0.24 - b * 0.04);
          if (alpha <= 0) continue;
          ctx.strokeStyle = `rgba(224, 247, 255, ${alpha})`;
          ctx.lineWidth = Math.max(1, 1.25 * dpr - b * 0.12 * dpr);
          ctx.beginPath();
          ctx.arc(bx, by, (2.1 + b * 0.45) * dpr, 0, Math.PI * 2);
          ctx.stroke();
        }
      }

      const pulse = (Math.sin(t * 2.2) + 1) / 2;
      const letterSize = Math.max(40, Math.floor(Math.min(w * 0.15, h * 0.24)));
      const subSize = Math.max(12, Math.floor(letterSize * 0.2));
      const textY = h * 0.42;
      const headline = "DON'T PANIC";

      ctx.save();
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';

      const glow = 0.28 + pulse * 0.4;
      ctx.shadowColor = `rgba(119, 222, 255, ${glow})`;
      ctx.shadowBlur = 22 * dpr;

      ctx.lineWidth = Math.max(2, letterSize * 0.055);
      ctx.strokeStyle = 'rgba(6, 22, 48, 0.72)';
      ctx.fillStyle = '#fff9de';
      ctx.font = `900 ${letterSize}px "Iowan Old Style", "Palatino Linotype", serif`;

      // Draw whole headline in layered jitter passes so kerning (including apostrophe) stays natural.
      const passCount = 5;
      for (let i = 0; i < passCount; i += 1) {
        const phase = t * (1.1 + i * 0.1);
        const wobbleX = Math.sin(phase + i * 0.7) * (1.4 + i * 0.22) * dpr;
        const wobbleY = Math.cos(phase * 1.07 + i * 0.5) * (1.1 + i * 0.18) * dpr;
        const tilt = Math.sin(phase * 0.6 + i) * 0.015;
        const alpha = 0.24 + i * 0.12;

        ctx.save();
        ctx.translate(w * 0.5 + wobbleX, textY + wobbleY);
        ctx.rotate(tilt);
        ctx.strokeStyle = `rgba(6, 22, 48, ${0.48 + i * 0.06})`;
        ctx.fillStyle = `rgba(255, 249, 222, ${Math.min(0.98, alpha)})`;
        ctx.strokeText(headline, 0, 0);
        ctx.fillText(headline, 0, 0);
        ctx.restore();
      }

      ctx.shadowBlur = 0;
      ctx.fillStyle = 'rgba(214, 241, 255, 0.95)';
      ctx.font = `700 ${subSize}px "Avenir Next", "Segoe UI", sans-serif`;
      ctx.fillText('in LARGE FRIENDLY LETTERS', w * 0.5, textY + letterSize * 0.52);

      ctx.restore();

      state.lockOverlayAnimationId = requestAnimationFrame(paint);
    };

    stopLockOverlayAnimation();
    state.lockOverlayStarDrift = 0;
    state.lockOverlayAnimationId = requestAnimationFrame(paint);
  }

  function showLockOverlay() {
    const overlay = byId('lock-overlay');
    if (!overlay) return;

    overlay.classList.remove('hidden');
    overlay.setAttribute('aria-hidden', 'false');
    overlay.focus();
    drawLockOverlayScene();
  }

  function onLockOverlayKeydown(event) {
    if (event.key === 'Escape' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      hideLockOverlay();
    }
  }

  function setGatewayStatus(message, kind = 'idle') {
    const el = byId('gateway-status');
    if (!el) return;
    el.textContent = message;
    el.className = `status ${kind}`;
  }

  function setGatewayInstallNoteVisible(visible, mode = 'install') {
    const note = byId('gateway-install-note');
    if (!note) return;

    const ipnsSubdomainMatch = String(window.location.hostname || '').toLowerCase().match(/^([^.]+)\.ipns\.localhost$/);
    const ipnsPathGatewayUrl = ipnsSubdomainMatch
      ? `http://localhost:${window.location.port || '8080'}/ipns/${ipnsSubdomainMatch[1]}/`
      : '';
    const origin = window.location.origin;
    const allowOrigins = Array.from(new Set([
      'http://127.0.0.1:8080',
      'http://localhost:8080',
      'http://127.0.0.1:8081',
      'http://localhost:8081',
      'http://127.0.0.1:8082',
      'http://localhost:8082',
      origin,
    ]));
    const allowOriginsJson = JSON.stringify(allowOrigins);
    const allowOriginsCmd = `ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin '${allowOriginsJson}'`;

    if (mode === 'origin-blocked') {
      note.innerHTML =
        `<p>IPFS RPC appears to run locally, but this page origin cannot call the configured local IPFS RPC API (typically <code>http://localhost:8080</code>).</p>` +
        `<p>Use IPFS Desktop (Settings -> IPFS Config) or IPFS RPC CLI (<code>ipfs config</code>) to merge the generated JSON file into your config:</p>` +
        `<p>Recommended: run the app from local runtime URL <code>http://127.0.0.1:8081</code> (or <code>http://localhost:8081</code>) when gateway origin blocks local API calls.</p>` +
        `<p><code>${origin}</code><button type="button" class="copy-origin" id="copy-gateway-origin">Copy</button></p>` +
        `<p>Open generated file from this app: <a href="./gateway-config.merge.json" target="_blank" rel="noreferrer"><code>gateway-config.merge.json</code></a></p>` +
        `<p>If Desktop editor ignores full JSON paste, edit only <code>Gateway.PublicGateways</code>. If it is <code>null</code>, replace that value with the object from <code>gateway-config.merge.json</code>.</p>`;
    } else if (mode === 'gateway-api-blocked') {
      note.innerHTML =
        `<p>IPFS RPC is running, but API calls from gateway origin <code>${origin}</code> are blocked by IPFS RPC security policy.</p>` +
        `<p>This is expected for pages opened from local gateway on port <code>8080</code>.</p>` +
        `<p>This app requires IPFS RPC API at runtime (identity publish + content resolution), so gateway-only mode is not sufficient.</p>` +
        `<p>Preferred fix: open local runtime URL <code>http://127.0.0.1:8081</code> (or <code>http://localhost:8081</code>).</p>` +
        `<p>Fallback: open local runtime URL <code>http://127.0.0.1:8081</code> (or <code>http://localhost:8081</code>).</p>`;
    } else if (mode === 'cors-blocked') {
      note.innerHTML =
        `<p>IPFS RPC likely runs, but CORS for this origin is missing:</p>` +
        `<p>Recommended: use local runtime URL <code>http://127.0.0.1:8081</code> while CORS is being configured.</p>` +
        `<p><code>${origin}</code><button type="button" class="copy-origin" id="copy-gateway-origin">Copy</button></p>` +
        `<p>Open generated file from this app: <a href="./gateway-config.merge.json" target="_blank" rel="noreferrer"><code>gateway-config.merge.json</code></a></p>` +
        `<p>Ensure these origins are included: <code>${allowOrigins.join('</code>, <code>')}</code></p>` +
        `<p>CLI quick-fix for origins:</p>` +
        `<p><code>${allowOriginsCmd}</code></p>` +
        `<p>Important: if this page is on <code>http://127.0.0.1:8080</code>, that exact origin must be present in IPFS RPC allow-origin.</p>` +
        `<p>Apply this via IPFS Desktop config or IPFS RPC CLI (<code>ipfs config</code>).</p>` +
        `<p>If <code>Gateway.PublicGateways</code> is <code>null</code>, replace it with the object from <code>gateway-config.merge.json</code> (do not paste over the entire config file).</p>` +
        `<p>To avoid subdomain behavior, open the app as <code>http://localhost:8080/ipns/&lt;key&gt;/</code>, not <code>http://&lt;key&gt;.ipns.localhost:8080/</code>.</p>` +
        (ipnsPathGatewayUrl
          ? `<p>Workaround: open the same app via localhost path gateway, which IPFS RPC accepts better for CORS:<br><a href="${ipnsPathGatewayUrl}"><code>${ipnsPathGatewayUrl}</code></a></p>`
          : '');
    } else if (mode === 'gateway-fallback') {
      const options = ipfsGatewayFallbacks
        .map((entry) => `<code>${entry}</code>`)
        .join(', ');
      note.innerHTML =
        `<p>Could not reach the configured IPFS gateway from this browser tab.</p>` +
        `<p>Try one of these gateway endpoints:</p>` +
        `<p>${options}</p>` +
        `<p>Recommended: keep <code>http://localhost:8080</code> as primary when available.</p>`;
    } else {
      note.innerHTML = '<p>IPFS RPC not available. If you have not installed it yet, download from <a href="https://docs.ipfs.tech/install/" target="_blank" rel="noreferrer">IPFS/IPFS RPC install docs</a>.</p>';
    }

    const copyBtn = byId('copy-gateway-origin');
    if (copyBtn) {
      copyBtn.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(window.location.origin);
          setSetupStatus(`Copied origin: ${window.location.origin}`);
        } catch {
          setSetupStatus(`Could not copy origin. Use: ${window.location.origin}`);
        }
      });
    }

    note.classList.toggle('hidden', !visible);
  }

  return {
    hideLockOverlay,
    showLockOverlay,
    onLockOverlayKeydown,
    setGatewayStatus,
    setGatewayInstallNoteVisible,
  };
}
