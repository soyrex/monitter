const taskId = "11111111-1111-4111-8111-111111111111";
const agentId = "22222222-2222-4222-8222-222222222222";
const oldMessage = {
  id: "old",
  taskId,
  role: "user",
  text: "Same text",
  createdAt: Date.now() - 2000,
};
const agentMessage = {
  id: "agent",
  taskId,
  role: "assistant",
  senderAgentId: agentId,
  text: "Atlas is here.",
  createdAt: Date.now() - 1500,
};
const state = {
  hosts: [],
  agents: [
    {
      id: agentId,
      name: "Atlas",
      description: "",
      instructions: "",
      avatar: null,
      provider: "codex",
      model: "",
      hostId: "",
      cwd: "",
      color: "#397e61",
      sandbox: "read-only",
      expertise: [],
      responsibilities: [],
      skills: [],
      collaborationEnabled: true,
    },
  ],
  tasks: [
    {
      id: taskId,
      agentId,
      title: "Shared design chat",
      nativeSessionId: null,
      status: "idle",
      archived: false,
      createdAt: Date.now() - 3000,
      updatedAt: Date.now(),
      parentTaskId: null,
      channelId: null,
      projectId: null,
      hostId: "",
      cwd: "",
      provider: "codex",
      model: "",
      sandbox: "read-only",
    },
  ],
  messages: [oldMessage, agentMessage],
  events: [],
  channels: [],
  projects: [],
  collaborations: [],
  queuedMessages: [],
  approvalRequests: [],
  settings: {
    accent: "#397e61",
    theme: "light",
    interfaceScale: 100,
    showToolActivity: true,
    showReasoningSummaries: true,
    sendWithEnter: false,
    sidebarView: "standard",
  },
  sharing: {
    primary: { name: "Alex", role: "primary user" },
    visitor: { name: "Riley", role: "visitor" },
    uploads: { maxFileBytes: 1024 * 1024, maxFiles: 2 },
    appearance: {
      theme: "dark",
      variables: {
        "--paper": "#242724",
        "--sidebar": "#191b19",
        "--panel": "#2d302d",
        "--line": "#4b504a",
        "--soft": "#343934",
        "--code": "#1d201d",
        "--ink": "#edf0ea",
        "--muted": "#a5ada3",
        "--accent": "#bd6d43",
        "--accent-rgb": "189, 109, 67",
        "--accent-ink": "#ffb58d",
        "--on-accent": "#ffffff",
      },
    },
  },
};
const clone = () => structuredClone(state);
export async function createMobileSession() {
  const api = globalThis.window?.__shareTest;
  if (api) api.snapshot = state;
  const listeners = new Set();
  const session = {
    getStatus: () => "connected",
    getVerificationCode: () => null,
    supportsRememberedDevices: () => false,
    subscribe(listener) {
      listeners.add(listener);
      queueMicrotask(() => listener({ status: "connected" }));
      return () => listeners.delete(listener);
    },
    async getSnapshot() {
      if (api) api.snapshotCalls += 1;
      return clone();
    },
    async storeAttachment(taskId, file) {
      if (api) api.uploaded = (api.uploaded || 0) + 1;
      if (file.filename === "reject.txt")
        throw new Error("simulated upload failure");
      return {
        id: `attachment-${api?.uploaded || 1}`,
        name: file.filename,
        mimeType: file.mimeType,
        size: Math.floor((file.dataBase64.length * 3) / 4),
        path: "Shared attachment",
      };
    },
    async sendMessage(taskId, text, attachmentIds = []) {
      if (api) {
        api.sendStarted = true;
        await new Promise((resolve) => {
          api.releaseSend = resolve;
        });
      }
      if (api?.failNext) {
        api.failNext = false;
        throw new Error("simulated transport failure");
      }
      state.messages.push({
        id: `new-${state.messages.length}`,
        taskId,
        role: "user",
        text: `@(${api?.visitorName || "Visitor"}): ${text}`,
        attachments: attachmentIds.map((id) => ({
          id,
          name: "upload.txt",
          mimeType: "text/plain",
          size: 5,
          path: "Shared attachment",
        })),
        createdAt: Date.now(),
      });
      return clone();
    },
    close() {},
    async cancelTask() {},
    async resumeTask() {},
    async listTerminals() {
      return [];
    },
    async readTerminal() {
      return { chunks: [] };
    },
  };
  return session;
}
