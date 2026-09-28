/** Distinguishes a lost owner transport from a desktop command failure. */
export class OwnerRelayDisconnectedError extends Error {
  constructor() {
    super('Reconnecting to your desktop. Try again once connected.');
    this.name = 'OwnerRelayDisconnectedError';
  }
}

/** @param {unknown} reason */
export const isOwnerRelayDisconnectedError = reason => reason instanceof OwnerRelayDisconnectedError;
