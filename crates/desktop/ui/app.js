const invoke = window.__TAURI__.core.invoke;
const form = document.querySelector("#turn");
const prompt = document.querySelector("#prompt");
const start = document.querySelector("#start");
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

function render() {
  const inOffice = view === "office";
  studio.setAttribute("aria-pressed", String(!inOffice));
  office.setAttribute("aria-pressed", String(inOffice));
  heading.textContent = inOffice ? "Agentes no Office" : "Sessões do Studio";
  list.classList.toggle("office", inOffice);
  const focused = document.activeElement?.dataset.session;
  const rows = snapshot.sessions.map((session) => {
    const row = document.createElement("li");
    const card = document.createElement("button");
    card.type = "button";
    card.dataset.session = session.id;
    card.setAttribute("aria-pressed", String(selected === session.id));
    if (inOffice) {
      const avatar = document.createElement("span");
      avatar.className = "avatar";
      avatar.setAttribute("aria-hidden", "true");
      card.append(avatar);
    }
    const title = document.createElement("strong");
    title.textContent = `#${session.id} · ${session.agent} · ${stateLabels[session.state] ?? session.state}`;
    const task = document.createElement("span");
    task.textContent = session.task;
    card.append(title, task);
    card.addEventListener("click", () => {
      selected = session.id;
      render();
    });
    row.append(card);
    return row;
  });
  if (!rows.length) {
    const empty = document.createElement("li");
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
  if (!session) return;
  document.querySelector("#detail-title").textContent = `Sessão #${session.id}`;
  const output = document.querySelector("#answer");
  output.textContent = session.task;
  if (session.state !== "completed" || session.agent !== "codex") return;
  try {
    const answer = await invoke("answer", { id: session.id });
    if (selected === session.id) output.textContent = answer;
  } catch (error) {
    if (selected === session.id) output.textContent = String(error);
  }
}

async function refresh() {
  const next = await invoke("snapshot");
  if (next.cursor > snapshot.cursor) {
    snapshot = next;
    render();
  }
}

async function poll() {
  try {
    await refresh();
  } catch (error) {
    status.textContent = String(error);
  }
  setTimeout(poll, 1000);
}

studio.addEventListener("click", () => { view = "studio"; render(); });
office.addEventListener("click", () => { view = "office"; render(); });
form.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (start.disabled || !prompt.value.trim()) return;
  start.disabled = true;
  status.textContent = "Turno em execução.";
  try {
    selected = await invoke("run_turn", { prompt: prompt.value.trim() });
    await refresh();
    render();
    status.textContent = `Sessão #${selected} concluída.`;
  } catch (error) {
    status.textContent = String(error);
  } finally {
    start.disabled = false;
  }
});

void poll();
