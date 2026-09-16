import { getContext, setContext } from 'svelte';

/**
 * MessagePane owns reader intent. The registered virtual list owns every
 * programmatic write to its scroll element.
 */
export type TranscriptScrollOwner = {
  scrollToLatest: () => void;
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
