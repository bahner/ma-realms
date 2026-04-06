export function createDialogWriter({ byId, displayActor, formatDialogText }) {
  function asDialogText(value) {
    const source = String(value || '');
    if (typeof formatDialogText === 'function') {
      return String(formatDialogText(source));
    }
    return source;
  }

  function appendMessage(role, message) {
    const transcript = byId('transcript');
    const row = document.createElement('div');
    row.className = `msg ${role}`;

    const text = document.createElement('p');
    text.textContent = asDialogText(message);

    row.appendChild(text);
    transcript.appendChild(row);
    transcript.scrollTop = transcript.scrollHeight;
  }

  function writeChat(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', `${actor}: ${text}`);
  }

  function writeWhisper(senderDid, senderHandle, text) {
    const actor = displayActor(senderDid, senderHandle);
    appendMessage('world', `${actor} whispers ${text}.`);
  }

  function writeSystem(text) {
    appendMessage('system', asDialogText(text));
  }

  function writeWorld(text) {
    appendMessage('world', asDialogText(text));
  }

  return {
    appendMessage,
    writeChat,
    writeWhisper,
    writeSystem,
    writeWorld
  };
}
