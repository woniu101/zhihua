import type { AspectRatio, CandidateQuality, GenerationMode, H3AudioPolicy } from "./storyboard";

export type GpuStartMode = "gpu" | "cpu_no_gpu";
export type GpuRuntimeState =
  | "unbound"
  | "stopped"
  | "starting"
  | "running_gpu"
  | "running_no_gpu"
  | "stopping"
  | "generating"
  | "insufficient_capacity"
  | "error";

export interface MoneyValue {
  amount: number | null;
  currency: "CNY";
  unit?: "hour" | "month" | "one_time";
  queriedAt: string;
}

export interface GpuInstance {
  id: string;
  name: string;
  region: string;
  zone: string;
  state: GpuRuntimeState;
  gpuModel: string;
  gpuCount: number;
  cpuCores: number;
  memoryGb: number;
  supportsNoGpu: boolean;
  startedAt?: string;
  stoppedAt?: string;
  releaseAt?: string;
  releaseAtConfirmed: boolean;
  stopDeadline?: string;
}

export interface GpuAccountSummary {
  available: MoneyValue;
  cash?: MoneyValue;
  grant?: MoneyValue;
  runningPrice?: MoneyValue;
  currentRunEstimate?: MoneyValue;
  stale: boolean;
}

export interface RuntimeCapabilities {
  serviceVersion: string;
  apiVersion: string;
  workflowVersion: string;
  modelManifestVersion: string;
  comfyUiReady: boolean;
  workflows: Array<"image_generate" | "image_edit" | "t2v" | "i2v" | "flf2v" | "r2v" | "seedvr2">;
  acceptedWorkflowIds?: string[];
  availableWorkflowIds?: string[];
}

export type GenerationJobStatus =
  | "queued"
  | "waiting_for_compute"
  | "preparing"
  | "uploading"
  | "running"
  | "upscaling"
  | "downloading"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export interface VideoGenerationRequest {
  clientRequestId: string;
  projectId: string;
  sceneId: string;
  mode: GenerationMode;
  quality: CandidateQuality;
  aspectRatio: AspectRatio;
  durationSec: 5 | 10 | 15;
  prompt: string;
  ambientSound: string;
  seed: number;
  assetIds: string[];
  h3AudioPolicy: H3AudioPolicy;
}

export interface ImageGenerationRequest {
  clientRequestId: string;
  projectId: string;
  sceneId: string;
  mode: "generate" | "edit";
  aspectRatio: AspectRatio;
  prompt: string;
  negativePrompt?: string;
  seed: number;
  sourceAssetId?: string;
}

export interface GenerationJob {
  id: string;
  clientRequestId: string;
  remotePromptId?: string;
  status: GenerationJobStatus;
  progress: number | null;
  stageMessage: string;
  createdAt: string;
  updatedAt: string;
  errorCode?: string;
  errorMessage?: string;
}

export interface GpuProvider {
  listInstances(): Promise<GpuInstance[]>;
  getInstance(instanceId: string): Promise<GpuInstance>;
  start(instanceId: string, mode: GpuStartMode): Promise<void>;
  stop(instanceId: string): Promise<void>;
  getAccountSummary(instanceId?: string): Promise<GpuAccountSummary>;
  setStopDeadline(instanceId: string, unixTime: number): Promise<void>;
}

export interface VideoProvider {
  getCapabilities(): Promise<RuntimeCapabilities>;
  submit(request: VideoGenerationRequest): Promise<GenerationJob>;
  getStatus(jobId: string): Promise<GenerationJob>;
  cancel(jobId: string): Promise<void>;
  downloadResult(jobId: string, destination: string): Promise<string>;
}

export interface ImageProvider {
  getCapabilities(): Promise<RuntimeCapabilities>;
  submit(request: ImageGenerationRequest): Promise<GenerationJob>;
  getStatus(jobId: string): Promise<GenerationJob>;
  cancel(jobId: string): Promise<void>;
}
