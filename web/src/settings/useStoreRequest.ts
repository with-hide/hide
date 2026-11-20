import { useCallback, useState } from "react";
import type { Actions } from "../actions";
import type { StoreAnswer, StoreChange } from "../snapshot";
import { useShellStore } from "../store";
import { answerPending, refusalOf, storeRequestId } from "./skillsMcp";

/**
 * One press's store request on one machine (`kit_store`) and what it came to,
 * read from `status.store_answers` by its id; the core keeps the answers, the
 * page only remembers which one it waits for.
 */
export function useStoreRequest(deviceId: string, actions: Pick<Actions, "changeStore">) {
  const [sent, setSent] = useState<string | null>(null);
  const answer = useShellStore((s): StoreAnswer | null => (sent ? (s.rest?.status?.store_answers?.find((row) => row.request_id === sent) ?? null) : null));
  const send = useCallback(
    (changes: StoreChange[]) => {
      const id = storeRequestId();
      setSent(id);
      actions.changeStore(deviceId, id, changes);
      return id;
    },
    [actions, deviceId],
  );
  const reset = useCallback(() => setSent(null), []);
  return { send, reset, sent, answer, pending: answerPending(sent, answer), refusal: refusalOf(answer), done: answer?.state === "done" && refusalOf(answer) === null };
}
