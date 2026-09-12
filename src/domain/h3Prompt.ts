import type { GenerationMode, H3AudioPolicy } from "./storyboard";

export const H3_PROMPT_COMPILER_VERSION = "h3-prompt-v1";

export interface H3PromptIntent {
  visualTimeline: string;
  ambientSound?: string;
  allowHumanVoice: false;
  nonDiegeticMusic: null;
}

export interface CompiledH3Prompt {
  text: string;
  version: typeof H3_PROMPT_COMPILER_VERSION;
  audioPolicy: H3AudioPolicy;
}

function clean(value: string): string {
  return value
    .replace(/<\/?d(?:\s[^>]*)?>/gi, "")
    .replace(/\r?\n+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function referenceContract(mode: GenerationMode, durationSec: number): string {
  if (mode === "i2v" || mode === "continue") {
    return "At 0.00 seconds, <Picture 1> is used as the exact opening frame and composition reference.";
  }
  if (mode === "flf2v") {
    return `At 0.00 seconds, <Picture 1> is the exact opening frame. At ${durationSec.toFixed(2)} seconds, <Picture 2> is the exact ending frame.`;
  }
  if (mode === "r2v") {
    return "Use <Video 1> as the motion reference and <Picture 1> as the character and scene appearance reference while preserving the requested target composition.";
  }
  return "Create the target video directly from the described shot.";
}

export function compileH3Prompt(input: {
  intent: H3PromptIntent;
  mode: GenerationMode;
  durationSec: 5 | 10 | 15;
  audioPolicy: H3AudioPolicy;
}): CompiledH3Prompt {
  const visual = clean(input.intent.visualTimeline);
  if (!visual) throw new Error("H3 画面时间线不能为空。");
  const ambient = clean(input.intent.ambientSound ?? "")
    || "Natural diegetic environmental and physical sounds that match only the visible actions.";
  const reference = referenceContract(input.mode, input.durationSec);
  const soundscape = input.audioPolicy === "off"
    ? "N/A. Complete silence."
    : `${ambient} No dialogue, narration, speech, whispering, singing, chanting, or other human vocalization.`;

  return {
    version: H3_PROMPT_COMPILER_VERSION,
    audioPolicy: input.audioPolicy,
    text: [
      `For the target video, ${reference}`,
      "",
      `integrated_multimodal_description: ${visual} No visible person speaks. Do not render subtitles, captions, labels, logos, or watermarks into the video.`,
      "",
      `overall_soundscape: ${soundscape}`,
      "",
      "non_diegetic_music: N/A.",
    ].join("\n"),
  };
}
