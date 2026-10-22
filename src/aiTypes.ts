export type AiRecommendation = {
  path: string;
  name: string;
  isDir: boolean;
  bytes: number | null;
  safety: "safe" | "caution";
  reason: string;
  exists: boolean;
};

export type AiQueryEvent =
  | { event: "status"; data: { message: string } }
  | { event: "chunk"; data: { text: string } }
  | {
      event: "done";
      data: { answer: string; recommendations: AiRecommendation[] };
    };

export type TrashOutcome = {
  originalPath: string;
  trashPath: string;
  freedBytes: number;
  putBack: boolean;
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  /** Who answered, kept so switching providers does not relabel earlier replies. */
  providerName?: string;
  modelLabel?: string;
  /** Set when the reply did not finish. Such replies are not sent back as history. */
  outcome?: "error" | "stopped";
  recommendations?: AiRecommendation[];
};
