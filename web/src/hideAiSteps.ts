// What each part of Hide AI gives the operator, in three steps each, as the
// owl says it wherever a tab of the Hide AI tool has nothing to show or is off.
import { BookmarkIcon, BotIcon, LightbulbIcon, MessagesSquareIcon, ScrollTextIcon, SparklesIcon } from "lucide-react";
import type { HideAiStep } from "./components/hide-ai-explainer";
import type { MessageKey } from "./i18n/catalogs";

type Translate = (key: MessageKey) => string;

export function suggestionSteps(t: Translate): HideAiStep[] {
  return [
    { icon: ScrollTextIcon, text: t("hideAiTool.explain.suggestions.find") },
    { icon: LightbulbIcon, text: t("hideAiTool.explain.suggestions.suggest") },
    { icon: SparklesIcon, text: t("hideAiTool.explain.suggestions.smarter") },
  ];
}

export function memorySteps(t: Translate): HideAiStep[] {
  return [
    { icon: MessagesSquareIcon, text: t("hideAiTool.explain.memory.remember") },
    { icon: BotIcon, text: t("hideAiTool.explain.memory.shared") },
    { icon: BookmarkIcon, text: t("hideAiTool.explain.memory.once") },
  ];
}
