import { getContext, setContext } from 'svelte';

/**
 * MessagePane owns reader intent. The registered virtual list owns every
 * programmatic write to its scroll element.
 *
 * `scrollToLatest` accepts an options bag:
 *   - `animate`: when true, smooth-scroll the viewport (respects
 *     prefers-reduced-motion natively). Streaming/follow callers should leave
 *     it false so live chats don't lag behind new tokens.
 */
export type ScrollToLatestOptions = { animate?: boolean };

export type TranscriptScrollOwner = {
  scrollToLatest: (options?: ScrollToLatestOptions) => void;
  isAtLatest: () => boolean;
  setFollowing: (following: boolean) => void;
};

type TranscriptScrollController = {
  register: (owner: TranscriptScrollOwner) => () => void;
  isFollowing: () => boolean;
};

const transcriptScrollController = Symbol('transcript-scroll-controller');

export function provideTranscriptScrollController(controller: TranscriptScrollController) {
  setContext(transcriptScrollController, controller);
}

export function useTranscriptScrollController() {
  return getContext<TranscriptScrollController | undefined>(transcriptScrollController);
}
