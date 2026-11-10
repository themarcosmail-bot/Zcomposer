const invoke = window.__TAURI__.core.invoke;
const form = document.querySelector("#turn");
const prompt = document.querySelector("#prompt");
const start = document.querySelector("#start");
const interrupt = document.querySelector("#interrupt");
const status = document.querySelector("#status");
const list = document.querySelector("#sessions");
const detail = document.querySelector("#detail");
const heading = document.querySelector("#heading");
const studio = document.querySelector("#studio");
const office = document.querySelector("#office");
const stateLabels = {
  queued: "Na fila",
  running: "Trabalhando",
  completed: "Concluída",
  failed: "Com erro",
  interrupted: "Interrompida",
};
let view = "studio";
let snapshot = { cursor: -1, sessions: [] };
let selected = null;
let studioOrder = [];
let dragged = null;
let savedOrder = [];
let orderSave = Promise.resolve();
let orderVersion = 0;
let detailSession = null;
let detailRequest = 0;
let interruptRequest = null;

function moveStudioCard(id, targetIndex) {
  const index = studioOrder.indexOf(id);
  if (index < 0 || targetIndex < 0 || targetIndex >= studioOrder.length || index === targetIndex) return;
  studioOrder.splice(index, 1);
  studioOrder.splice(targetIndex, 0, id);
  render();
  document.querySelector("#layout-status").textContent =
    `Sessão #${id} movida para a posição ${targetIndex + 1}.`;
  const version = ++orderVersion;
  const ids = studioOrder.slice();
  const saveStatus = document.querySelector("#layout-save");
  saveStatus.textContent = "Salvando a ordem…";
  orderSave = orderSave.then(async () => {
    try {
      await invoke("save_studio_order", { ids });
      savedOrder = ids;
      if (version === orderVersion) saveStatus.textContent = "";
    } catch {
      if (version !== orderVersion) return;
      studioOrder = savedOrder.slice();
      dragged = null;
      render();
      saveStatus.textContent = "Falha ao salvar a ordem. A última ordem salva foi restaurada.";
    }
  });
}

function renderStudioControls(row, id, index) {
  row.addEventListener("dragover", (event) => {
    if (dragged === null || dragged === id) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  });
  row.addEventListener("drop", (event) => {
    if (dragged === null) return;
    event.preventDefault();
    const source = dragged;
    dragged = null;
    moveStudioCard(source, index);
  });

  const controls = document.createElement("div");
  controls.className = "studio-controls";
  const handle = document.createElement("span");
  handle.className = "drag-handle";
  handle.textContent = "⠿ Arrastar";
  handle.draggable = true;
  handle.addEventListener("dragstart", (event) => {
    dragged = id;
    event.dataTransfer.setData("text/plain", String(id));
    event.dataTransfer.effectAllowed = "move";
  });
  handle.addEventListener("dragend", () => { dragged = null; });
  controls.append(handle);
  for (const [offset, icon, label] of [[-1, "←", "Mover antes"], [1, "→", "Mover depois"]]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = icon;
    button.setAttribute("aria-label", `${label} a sessão #${id}`);
    button.setAttribute("aria-disabled", String(index + offset < 0 || index + offset >= studioOrder.length));
    button.addEventListener("click", () => {
      if (button.getAttribute("aria-disabled") === "true") return;
      moveStudioCard(id, index + offset);
      list.querySelector(`[data-session="${id}"]`)?.focus();
    });
    controls.append(button);
  }
  row.append(controls);
}

function render() {
  const inOffice = view === "office";
  studio.setAttribute("aria-pressed", String(!inOffice));
  office.setAttribute("aria-pressed", String(inOffice));
  heading.textContent = inOffice ? "Agentes no Office" : "Sessões do Studio";
  document.querySelector("#board-help").textContent = inOffice
    ? "Selecione uma estação para acompanhar a tarefa e o histórico."
    : "Arraste os cartões entre posições ou use as setas para reorganizá-los.";
  document.querySelector("#session-count").textContent = `Sessões: ${snapshot.sessions.length}`;
  list.className = inOffice ? "office" : "studio";
  const focused = document.activeElement?.dataset.session;
  const sessionsById = new Map(snapshot.sessions.map((session) => [session.id, session]));
  studioOrder = studioOrder.filter((id) => sessionsById.has(id));
  const orderedIds = new Set(studioOrder);
  for (const { id } of snapshot.sessions) {
    if (!orderedIds.has(id)) studioOrder.push(id);
  }
  const sessions = inOffice ? snapshot.sessions : studioOrder.map((id) => sessionsById.get(id));
  const rows = sessions.map((session, index) => {
    const row = document.createElement("li");
    row.className = "session";
    row.dataset.state = session.state;
    const card = document.createElement("button");
    card.type = "button";
    card.className = "session-card";
    card.dataset.session = session.id;
    card.setAttribute("aria-pressed", String(selected === session.id));
    if (inOffice) {
      const station = document.createElement("span");
      station.className = "station";
      station.setAttribute("aria-hidden", "true");
      const avatar = document.createElement("span");
      avatar.className = "avatar";
      station.append(avatar);
      card.append(station);
    }
    const title = document.createElement("strong");
    title.textContent = `#${session.id} · ${session.agent}`;
    const badge = document.createElement("span");
    badge.className = "state-badge";
    badge.textContent = stateLabels[session.state] ?? session.state;
    const task = document.createElement("span");
    task.textContent = session.task;
    task.className = "session-task";
    card.append(title, badge, task);
    card.addEventListener("click", () => {
      selected = session.id;
      render();
    });
    row.append(card);
    if (!inOffice) renderStudioControls(row, session.id, index);
    return row;
  });
  if (!rows.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "Nenhuma sessão neste projeto.";
    rows.push(empty);
  }
  list.replaceChildren(...rows);
  if (focused) list.querySelector(`[data-session="${focused}"]`)?.focus();
  void renderDetail();
}

async function renderDetail() {
  const session = snapshot.sessions.find(({ id }) => id === selected);
  detail.hidden = !session;
  if (session === detailSession) return;
  if (session && detailSession?.id === session.id && detailSession.state === session.state) return;
  detailSession = session;
  const request = ++detailRequest;
  if (!session) return;
  document.querySelector("#detail-title").textContent = `Sessão #${session.id}`;
  document.querySelector("#detail-task").textContent = session.task;
  const history = document.querySelector("#session-history");
  const historyError = document.querySelector("#history-error");
  history.replaceChildren();
  historyError.textContent = "";
  const output = document.querySelector("#answer");
  const hasAnswer = session.state === "completed" && session.agent === "codex";
  const isLive = session.state === "running" && session.agent === "codex";
  const answerHeading = document.querySelector("#answer-heading");
  answerHeading.hidden = !hasAnswer && !isLive;
  answerHeading.textContent = isLive ? "Resposta em andamento" : "Resposta";
  output.hidden = answerHeading.hidden;
  output.textContent = isLive ? "Aguardando resposta…" : hasAnswer ? "Carregando resposta…" : "";
  const [states, answer] = await Promise.allSettled([
    invoke("session_history", { id: session.id }),
    hasAnswer ? invoke("answer", { id: session.id }) : null,
  ]);
  if (request !== detailRequest) return;
  if (states.status === "fulfilled") {
    for (const state of states.value) {
      const entry = document.createElement("li");
      entry.textContent = stateLabels[state] ?? state;
      history.append(entry);
    }
  } else {
    historyError.textContent = "Não foi possível carregar o histórico.";
  }
  if (hasAnswer) {
    output.textContent = answer.status === "fulfilled" ? answer.value : "Não foi possível carregar a resposta.";
  }
}

async function refreshLiveAnswer() {
  const session = detailSession;
  if (session?.agent !== "codex" || session.state !== "running") return;
  const request = detailRequest;
  const text = await invoke("live_answer", { id: session.id });
  if (request === detailRequest && text) {
    document.querySelector("#answer").textContent = text;
  }
}

async function refresh() {
  const next = await invoke("snapshot");
  if (snapshot.cursor === -1) {
    const ids = await invoke("studio_order");
    if (snapshot.cursor === -1) {
      studioOrder = ids.slice();
      savedOrder = ids;
    }
  }
  if (next.cursor > snapshot.cursor && dragged === null) {
    snapshot = next;
    render();
  }
}

async function poll() {
  try {
    await refresh();
    await refreshLiveAnswer();
  } catch (error) {
    status.textContent = String(error);
  }
  setTimeout(poll, 1000);
}

document.addEventListener("dragover", (event) => {
  if (dragged === null) event.preventDefault();
});
document.addEventListener("drop", (event) => event.preventDefault());
studio.addEventListener("click", () => { view = "studio"; render(); });
office.addEventListener("click", () => { view = "office"; render(); });
interrupt.addEventListener("click", () => {
  if (interruptRequest || interrupt.disabled) return;
  interrupt.disabled = true;
  status.textContent = "Solicitando interrupção…";
  interruptRequest = invoke("interrupt_turn")
    .then((accepted) => {
      if (!accepted) status.textContent = "Nenhum turno ativo para interromper.";
      return accepted;
    })
    .catch(() => {
      status.textContent = "Não foi possível solicitar a interrupção.";
      return false;
    })
    .then((accepted) => {
      if (!accepted) {
        interruptRequest = null;
        interrupt.disabled = false;
      }
    });
});
form.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (start.disabled || !prompt.value.trim()) return;
  start.disabled = true;
  interrupt.hidden = false;
  status.textContent = "Turno em execução.";
  try {
    const turn = await invoke("run_turn", { prompt: prompt.value.trim() });
    interrupt.hidden = true;
    await interruptRequest;
    selected = turn.id;
    await refresh();
    render();
    const result = turn.state === "interrupted" ? "interrompida" : "concluída";
    status.textContent = `Sessão #${turn.id} ${result}.`;
  } catch (error) {
    interrupt.hidden = true;
    await interruptRequest;
    status.textContent = String(error);
  } finally {
    interruptRequest = null;
    interrupt.disabled = false;
    start.disabled = false;
  }
});

void poll();
