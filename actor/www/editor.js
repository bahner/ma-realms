export function createEditorUi({ byId, state, uiText, onEditorEngineStatus }) {
  let cmView = null;
  let cmContainer = null;
  const CODEMIRROR_MAJOR = '6';
  const CODEMIRROR_YAML_VERSION = '6.1.3';
  const CODEMIRROR_CDN_NAME = 'esm.sh';
  const CODEMIRROR_URLS = {
    state: `https://esm.sh/@codemirror/state@${CODEMIRROR_MAJOR}`,
    view: `https://esm.sh/@codemirror/view@${CODEMIRROR_MAJOR}`,
    commands: `https://esm.sh/@codemirror/commands@${CODEMIRROR_MAJOR}`,
    yaml: `https://esm.sh/@codemirror/lang-yaml@${CODEMIRROR_YAML_VERSION}`,
  };

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
      const stateMod = await import(CODEMIRROR_URLS.state);
      const viewMod = await import(CODEMIRROR_URLS.view);
      const commandsMod = await import(CODEMIRROR_URLS.commands);
      const { EditorState } = stateMod;
      const { EditorView, keymap } = viewMod;
      const { defaultKeymap } = commandsMod;
      if (!EditorState || !EditorView || !keymap || !defaultKeymap) {
        throw new Error('CodeMirror core mangler nødvendige exports');
      }

      let yamlExtension = null;
      let yamlLoadDetail = '';
      try {
        const yamlMod = await import(CODEMIRROR_URLS.yaml);
        if (typeof yamlMod?.yaml === 'function') {
          yamlExtension = yamlMod.yaml();
        } else {
          throw new Error('yaml() ikke tilgjengelig i @codemirror/lang-yaml');
        }
      } catch (yamlError) {
        yamlLoadDetail = yamlError instanceof Error ? yamlError.message : String(yamlError || 'unknown error');
      }

      cmContainer = document.createElement('div');
      cmContainer.id = 'yaml-editor-cm';
      cmContainer.className = 'yaml-editor-cm hidden';
      textEl.insertAdjacentElement('afterend', cmContainer);
      textEl.classList.add('hidden-by-cm');

      const baseExtensions = [
        keymap.of(defaultKeymap),
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (update.docChanged && textEl) {
            textEl.value = update.state.doc.toString();
          }
        }),
      ];
      const withYamlExtensions = yamlExtension ? [...baseExtensions, yamlExtension] : baseExtensions;

      try {
        cmView = new EditorView({
          parent: cmContainer,
          state: EditorState.create({
            doc: String(textEl.value || ''),
            extensions: withYamlExtensions,
          }),
        });
      } catch (viewError) {
        const viewDetail = viewError instanceof Error ? viewError.message : String(viewError || 'unknown error');
        const isStateMismatch = /Unrecognized extension value/i.test(viewDetail);
        if (yamlExtension && isStateMismatch) {
          // Known CDN module graph issue: YAML extension may carry a different @codemirror/state instance.
          cmView = new EditorView({
            parent: cmContainer,
            state: EditorState.create({
              doc: String(textEl.value || ''),
              extensions: baseExtensions,
            }),
          });
          yamlLoadDetail = yamlLoadDetail || viewDetail;
        } else {
          throw viewError;
        }
      }

      if (state.editBusy) {
        setEditorDisabled(true);
      }

      if (yamlLoadDetail) {
        notifyEditorEngine(
          `Editor: CodeMirror (${CODEMIRROR_CDN_NAME}) aktiv uten YAML-syntax (${yamlLoadDetail}). Bruker fortsatt rik editor.`
        );
      } else {
        notifyEditorEngine(`Editor: CodeMirror (${CODEMIRROR_CDN_NAME}) aktiv med YAML-syntax.`);
      }
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error || 'unknown error');
      notifyEditorEngine(
        `Editor: CodeMirror fra ${CODEMIRROR_CDN_NAME} kunne ikke lastes (${detail}). Forsøkte: ${Object.values(CODEMIRROR_URLS).join(' , ')}. Fortsetter med innebygd textarea-fallback.`
      );
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
    // Lazy-load CodeMirror only when the editor is actually used.
    initEditorEngineFromCdn().catch(() => {});
    setEditorDisabled(state.editBusy);
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
