// The agent's own question in a conversation's history (mockup v3 sections 2, 4): what it asked, by its question mark,
// and under each question the operator's answer once it came. The card the operator answers in stands at the
// conversation's foot (`SessionThreadAsk`); this is the line the question leaves behind.

import { CornerDownRightIcon } from "lucide-react";
import { useInterfaceTranslation } from "../../i18n/translator";
import type { QuestionLine } from "./rows";
import { ToolMarkIcon } from "./tool-lines";

/** Each question in a gray line under its mark, and its answer under it: the operator's words, else that it waits or was not answered. */
export function QuestionLineView({ line }: { line: QuestionLine }) {
  const { t } = useInterfaceTranslation();
  const asking = line.status === "pending" || line.status === "in_progress";
  return (
    <div className="flex min-w-0 flex-col gap-xs text-body" data-conversation-question={line.status}>
      {line.questions.map((question, at) => (
        <div key={at} className="flex min-w-0 flex-col gap-xxs">
          <p className="flex min-w-0 items-start gap-xs text-muted-foreground">
            <span className="flex h-lh shrink-0 items-center"><ToolMarkIcon mark="question" /></span>
            <span className="min-w-0 whitespace-pre-wrap break-keep text-subtle-foreground [overflow-wrap:anywhere]">{question.text}</span>
          </p>
          <p className="flex min-w-0 items-start gap-xs pl-(--size-icon-sm) ml-xs" data-question-answer={question.answer === null ? "none" : "answered"}>
            <span className="flex h-lh shrink-0 items-center"><CornerDownRightIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" /></span>
            {question.answer !== null ? (
              <span className="min-w-0 whitespace-pre-wrap break-keep text-foreground [overflow-wrap:anywhere]">{question.answer}</span>
            ) : (
              <span className="text-muted-foreground">{t(asking ? "conversation.question.waiting" : "conversation.question.unanswered")}</span>
            )}
          </p>
        </div>
      ))}
    </div>
  );
}
