import type { Message, Task } from '$lib/types';

export interface CompanionReplyPopup {
  message: Message | null;
  remaining: number;
}

/** Only completed agent replies can enter the companion's notification inbox. */
function finalReplies(messages: Message[], tasks: Task[]): Message[] {
  const eligible = new Map(tasks.filter(task => !task.archived && !task.channelId).map(task => [task.id, task]));
  const latest = new Map<string, Message>();
  for (const message of messages) {
    const previous = latest.get(message.taskId);
    if (!previous || message.createdAt >= previous.createdAt) latest.set(message.taskId, message);
  }
  return messages.filter(message => {
    const task = eligible.get(message.taskId);
    if (!task || message.role !== 'assistant' || !message.text.trim() ||
      message.streamStatus === 'streaming' || message.streamStatus === 'interrupted') return false;
    if (message.phase === 'final_answer') return true;
    return !message.phase && task.status === 'completed' && latest.get(task.id)?.id === message.id;
  }).sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id));
}

/** Presentation only: notifications never select a chat or authorize a send. */
export function createCompanionReplyInbox() {
  const seen = new Set<string>();
  let initialized = false;
  let queue: string[] = [];
  let replies = new Map<string, Message>();

  function snapshot(): CompanionReplyPopup {
    return { message: replies.get(queue[0]) ?? null, remaining: Math.max(0, queue.length - 1) };
  }

  return {
    sync(messages: Message[], eligibleTasks: Task[], initialTaskId: string | null): CompanionReplyPopup {
      const finals = finalReplies(messages, eligibleTasks);
      replies = new Map(finals.map(message => [message.id, message]));
      queue = queue.filter(id => replies.has(id));
      if (!initialized) {
        const latestSelected = [...finals].reverse().find(message => message.taskId === initialTaskId);
        if (latestSelected) queue.push(latestSelected.id);
        initialized = true;
      } else {
        for (const message of finals) {
          if (!seen.has(message.id)) { queue.push(message.id); seen.add(message.id); }
        }
      }
      for (const message of finals) seen.add(message.id);
      return snapshot();
    },
    dismiss(): CompanionReplyPopup {
      queue.shift();
      return snapshot();
    },
  };
}
