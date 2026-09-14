import type { Attachment } from '$lib/types';

/** Window-local delivery record; it is never persisted in Snapshot. */
export type OptimisticMessage = {
  id: string;
  kind: 'task' | 'channel' | 'draft';
  targetId: string;
  text: string;
  displayText: string;
  attachments: Attachment[];
  createdAt: number;
  status: 'sending' | 'sent' | 'not-confirmed';
  error?: string;
  baselineIds: Set<string>;
  baselineQueuedIds: Set<string>;
  recipientIds?: string[];
};
