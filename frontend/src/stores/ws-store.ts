import { create } from "zustand";
import type { WsMessage } from "@/types/api";

type WsStatus = "connecting" | "connected" | "disconnected";

interface WsState {
  status: WsStatus;
  lastEvent: WsMessage | null;
  eventCount: number;
  setStatus: (status: WsStatus) => void;
  onMessage: (msg: WsMessage) => void;
}

export const useWsStore = create<WsState>((set) => ({
  status: "disconnected",
  lastEvent: null,
  eventCount: 0,
  setStatus: (status) => set({ status }),
  onMessage: (msg) =>
    set((state) => ({
      lastEvent: msg,
      eventCount: state.eventCount + 1,
    })),
}));
