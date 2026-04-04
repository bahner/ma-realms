export function createEditorUi({ byId, state, uiText, onEditorEngineStatus }) {
  let cmView = null;
  let cmContainer = null;

  function notifyEditorEngine(message) {
    if (typeof onEditorEngineStatus === 'function') {
      onEditorEngineStatus(String(message || ''));
    }
  }

  async function initEditorEngineFromCdn() {
    const textEl = byId('yaml-editor-text');
    if (!textEl || cmView) {
      return;
    }

    try {
      const [stateMod, viewMod, commandsMod, languageMod, yamlMod] = await Promise.all([
        import('https://cdn.jsdelivr.net/npm/@codemirror/state@6/+esm'),
        import('https://cdn.jsdelivr.net/npm/@codemirror/view@6/+esm'),
        import('https://cdn.jsdelivr.net/npm/@codemirror/commands@6/+esm'),
        import('https://cdn.jsdelivr.net/npm/@codemirror/language@6/+esm'),
        import('https://cdn.jsdelivr.net/npm/@codemirror/lang-yaml@6/+esm'),
      ]);

      const { EditorState } = stateMod;
      const { EditorView, keymap, drawSelection, highlightActiveLine } = viewMod;
      const { history, historyKeymap, defaultKeymap, indentWithTab } = commandsMod;
      const { indentOnInput, bracketMatching, foldGutter } = languageMod;
      const { yaml } = yamlMod;

      cmContainer = document.createElement('div');
      cmContainer.id = 'yaml-editor-cm';
      cmContainer.className = 'yaml-editor-cm hidden';
      textEl.insertAdjacentElement('afterend', cmContainer);
      textEl.classList.add('hidden-by-cm');

      cmView = new EditorView({
        parent: cmContainer,
        state: EditorState.create({
          doc: String(textEl.value || ''),
          extensions: [
            keymap.of([indentWithTab, ...defaultKeymap, ...historyKeymap]),
            history(),
            drawSelection(),
            highlightActiveLine(),
            indentOnInput(),
            bracketMatching(),
            foldGutter(),
            yaml(),
            EditorView.lineWrapping,
            EditorView.updateListener.of((update) => {
              if (update.docChanged && textEl) {
                textEl.value = update.state.doc.toString();
              }
            }),
          ],
        }),
      });

      if (state.editBusy) {
        setEditorDisabled(true);
      }

      notifyEditorEngine('Editor: CodeMirror (CDN) active.');
    } catch (_) {
      notifyEditorEngine('Editor: CodeMirror CDN unavailable, using textarea fallback.');
    }
  }

  function isCodeMirrorActive() {
    return Boolean(cmView && cmContainer);
  }

  function setEditorText(value) {
    const text = String(value || '');
    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.value = text;
    }

    if (cmView) {
      const current = cmView.state.doc.toString();
      if (current !== text) {
        cmView.dispatch({
          changes: { from: 0, to: cmView.state.doc.length, insert: text },
        });
      }
    }
  }

  function getEditorText() {
    if (cmView) {
      return cmView.state.doc.toString();
    }
    const textEl = byId('yaml-editor-text');
    return String(textEl?.value || '');
  }

  function focusEditor() {
    if (cmView) {
      cmView.focus();
      return;
    }
    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.focus();
    }
  }

  function setEditorDisabled(disabled) {
    const isDisabled = Boolean(disabled);
    const textEl = byId('yaml-editor-text');
    if (textEl) {
      textEl.disabled = isDisabled;
    }

    if (cmContainer) {
      cmContainer.classList.toggle('disabled', isDisabled);
    }

    if (cmView) {
      cmView.contentDOM.setAttribute('aria-disabled', isDisabled ? 'true' : 'false');
      cmView.contentDOM.setAttribute('contenteditable', isDisabled ? 'false' : 'true');
    }
  }

  function onEditorModalVisibility(visible) {
    if (!cmContainer) return;
    cmContainer.classList.toggle('hidden', !visible);
  }

  function updateEditorContext() {
    const contextEl = byId('yaml-editor-context');
    if (!contextEl) return;
    if (!state.editSession) {
      contextEl.textContent = uiText('No edit target loaded.', 'Ingen redigeringsmål lastet.');
      return;
    }

    if (state.editSession.mode === 'script') {
      const cid = String(state.editSession.sourceCid || '').trim();
      if (cid && cid !== '(not published yet)') {
        contextEl.textContent = uiText(
          `Mode: local script | CID: ${cid}`,
          `Modus: lokalt script | CID: ${cid}`
        );
      } else {
        contextEl.textContent = uiText(
          'Mode: local script | CID: (not published yet)',
          'Modus: lokalt script | CID: (ikke publisert ennå)'
        );
      }
      return;
    }

    if (state.editSession.mode === 'avatar') {
      contextEl.textContent = uiText('Mode: avatar (@me)', 'Modus: avatar (@me)');
      return;
    }

    if (state.editSession.mode === 'exit') {
      contextEl.textContent = uiText(
        `Mode: exit | Target: ${state.editSession.target} | Source CID: ${state.editSession.sourceCid}`,
        `Modus: utgang | Mål: ${state.editSession.target} | Kilde-CID: ${state.editSession.sourceCid}`
      );
      return;
    }

    contextEl.textContent = uiText(
      `Mode: room | Target: ${state.editSession.target} | Source CID: ${state.editSession.sourceCid}`,
      `Modus: rom | Mål: ${state.editSession.target} | Kilde-CID: ${state.editSession.sourceCid}`
    );
  }

  function updateEditorControls() {
    const saveBtn = byId('yaml-editor-save');
    const reloadBtn = byId('yaml-editor-reload');
    const closeEvalBtn = byId('yaml-editor-close-eval');
    const textEl = byId('yaml-editor-text');
    if (!saveBtn || !reloadBtn || !textEl || !closeEvalBtn) return;
    const isNb = state.uiLang === 'nb';

    const mode = state.editSession?.mode || 'room';
    if (mode === 'script') {
      saveBtn.textContent = isNb ? 'Lagre lokalt script' : 'Save Local Script';
      reloadBtn.textContent = isNb ? 'Last lokalt' : 'Reload Local';
      closeEvalBtn.textContent = isNb ? 'Lukk og Evaluer' : 'Close and Eval';
      closeEvalBtn.classList.remove('hidden');
      textEl.placeholder = isNb ? 'Skriv lokal scripttekst her.' : 'Write local script text here.';
      return;
    }

    if (mode === 'avatar') {
      saveBtn.textContent = isNb ? 'Bruk avatar' : 'Apply Avatar';
      reloadBtn.textContent = isNb ? 'Last avatar' : 'Reload Avatar';
      closeEvalBtn.classList.add('hidden');
      textEl.placeholder = isNb ? 'Avatar-utkast (YAML-liknende).' : 'Avatar YAML-like draft.';
      return;
    }

    if (mode === 'exit') {
      saveBtn.textContent = isNb ? 'Lagre utgang' : 'Save Exit';
      reloadBtn.textContent = isNb ? 'Last utgang' : 'Reload Exit';
      closeEvalBtn.classList.add('hidden');
      textEl.placeholder = isNb ? 'Utgang-YAML vises her.' : 'Exit YAML will appear here.';
      return;
    }

    saveBtn.textContent = isNb ? 'Lagre + Publiser' : 'Save + Publish';
    reloadBtn.textContent = isNb ? 'Last kilde' : 'Reload Source';
    closeEvalBtn.classList.add('hidden');
    textEl.placeholder = isNb ? 'Rom-YAML vises her.' : 'Room YAML will appear here.';
  }

  function setEditorBusy(busy) {
    state.editBusy = Boolean(busy);
    for (const id of ['yaml-editor-reload', 'yaml-editor-cancel', 'yaml-editor-save', 'yaml-editor-close-eval']) {
      const el = byId(id);
      if (el) {
        el.disabled = state.editBusy;
      }
    }
    setEditorDisabled(state.editBusy);
  }

  function setEditorStatus(message, tone = 'idle') {
    const statusEl = byId('yaml-editor-status');
    if (!statusEl) return;
    statusEl.textContent = String(message || '');
    statusEl.classList.remove('ok', 'error', 'working');
    if (tone === 'ok' || tone === 'error' || tone === 'working') {
      statusEl.classList.add(tone);
    }
  }

  function closeEditorModal() {
    const modal = byId('yaml-editor-modal');
    if (!modal) return;
    modal.classList.add('hidden');
    modal.setAttribute('aria-hidden', 'true');
    onEditorModalVisibility(false);
    setEditorBusy(false);
    const input = byId('command-input');
    if (input) input.focus();
  }

  function openEditorModal() {
    const modal = byId('yaml-editor-modal');
    if (!modal) return;
    updateEditorContext();
    updateEditorControls();
    modal.classList.remove('hidden');
    modal.setAttribute('aria-hidden', 'false');
    onEditorModalVisibility(true);
    setTimeout(() => {
      focusEditor();
    }, 0);
  }

  function onEditorModalKeyDown(event) {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeEditorModal();
    }
  }

  return {
    initEditorEngineFromCdn,
    isCodeMirrorActive,
    setEditorText,
    getEditorText,
    focusEditor,
    setEditorDisabled,
    onEditorModalVisibility,
    openEditorModal,
    closeEditorModal,
    onEditorModalKeyDown,
    setEditorBusy,
    setEditorStatus,
    updateEditorContext,
    updateEditorControls,
  };
}
