/** Shared so callers can recognize this failure even after it has been
 * flattened to a plain string (Error -> message) by generic error formatting. */
export const OWNER_RELAY_DISCONNECTED_MESSAGE = 'Reconnecting to your desktop. Try again once connected.';

/** Distinguishes a lost owner transport from a desktop command failure. */
export class OwnerRelayDisconnectedError extends Error {
  constructor() {
    super(OWNER_RELAY_DISCONNECTED_MESSAGE);
    this.name = 'OwnerRelayDisconnectedError';
  }
}

/** @param {unknown} reason */
export const isOwnerRelayDisconnectedError = reason => reason instanceof OwnerRelayDisconnectedError;
