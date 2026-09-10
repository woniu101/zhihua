import { invokeNative } from "./nativeBridge";
import type { ServiceProbe } from "./serviceRepository";

export type TunnelPhase =
  | "stopped"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "error";

export interface TunnelStatus {
  configured: boolean;
  phase: TunnelPhase;
  localUrl?: string;
  lastError?: string;
}

const unavailable: TunnelStatus = {
  configured: false,
  phase: "stopped",
};

export const sshTunnelRepository = {
  status: async () =>
    (await invokeNative<TunnelStatus>("get_ssh_tunnel_status")) ?? unavailable,
  connectService: () =>
    invokeNative<ServiceProbe>("connect_service_through_tunnel"),
  stop: () => invokeNative<TunnelStatus>("stop_ssh_tunnel"),
};
