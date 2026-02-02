import { useCallback, useRef, useState } from "react";

import type { StreamChunk, StreamEventV1, StreamV1Format } from "@ditto-llm/client";
import { streamV1FromResponse } from "@ditto-llm/client";

export interface StreamV1State {
  text: string;
  chunks: StreamChunk[];
  responseId?: string;
  finishReason?: unknown;
  usage?: unknown;
  warnings: unknown[];
  error?: string;
  done: boolean;
  isLoading: boolean;
}

function extractFinishReason(chunk: StreamChunk): unknown | undefined {
  if (chunk.type !== "finish_reason") return undefined;
  if ("data" in chunk) return (chunk as any).data;
  if ("finish_reason" in chunk) return (chunk as any).finish_reason;
  if ("value" in chunk) return (chunk as any).value;
  return undefined;
}

function extractUsage(chunk: StreamChunk): unknown | undefined {
  if (chunk.type !== "usage") return undefined;
  if ("data" in chunk) return (chunk as any).data;
  return chunk;
}

export function useStreamV1() {
  const [state, setState] = useState<StreamV1State>({
    text: "",
    chunks: [],
    warnings: [],
    done: false,
    isLoading: false,
  });

  const abortRef = useRef<AbortController | null>(null);

  const abort = useCallback(() => {
    abortRef.current?.abort();
  }, []);

  const start = useCallback(
    async (createResponse: (signal: AbortSignal) => Promise<Response>, format: StreamV1Format) => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      setState({
        text: "",
        chunks: [],
        warnings: [],
        done: false,
        isLoading: true,
      });

      let res: Response;
      try {
        res = await createResponse(controller.signal);
      } catch (err) {
        setState((prev) => ({
          ...prev,
          isLoading: false,
          done: true,
          error: err instanceof Error ? err.message : String(err),
        }));
        return;
      }

      try {
        for await (const evt of streamV1FromResponse(res, format)) {
          const event = evt as StreamEventV1;
          if (event.type === "chunk") {
            const chunk = (event.data ?? {}) as StreamChunk;
            setState((prev) => {
              const next: StreamV1State = {
                ...prev,
                chunks: prev.chunks.concat(chunk),
              };

              if (chunk.type === "text_delta" && typeof (chunk as any).text === "string") {
                next.text = prev.text + (chunk as any).text;
              }
              if (chunk.type === "warnings" && Array.isArray((chunk as any).warnings)) {
                next.warnings = prev.warnings.concat((chunk as any).warnings);
              }
              if (chunk.type === "response_id" && typeof (chunk as any).id === "string") {
                next.responseId = (chunk as any).id;
              }

              const finishReason = extractFinishReason(chunk);
              if (finishReason !== undefined) next.finishReason = finishReason;

              const usage = extractUsage(chunk);
              if (usage !== undefined) next.usage = usage;

              return next;
            });
          } else if (event.type === "error") {
            setState((prev) => ({
              ...prev,
              error: event.data?.message ?? "unknown error",
            }));
          } else if (event.type === "done") {
            setState((prev) => ({ ...prev, done: true, isLoading: false }));
          }
        }
      } catch (err) {
        setState((prev) => ({
          ...prev,
          isLoading: false,
          done: true,
          error: err instanceof Error ? err.message : String(err),
        }));
      }
    },
    [],
  );

  return { state, start, abort };
}
