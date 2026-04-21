import { createContext, ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { listen } from "@tauri-apps/api/event";
import { SophonProgress, Variants } from "../types";
import { getGameName } from "../util/AppFunctions";

export type DownloadPhase = "idle" | "fetchingManifest" | "downloading" | "assembling" | "verifying" | "paused" | "finished" | "error";

export interface DownloadState {
  phase: DownloadPhase;
  downloadingGame: Variants | null;
  downloadedBytes: number;
  downloadTotal: number;
  speedBps: number;
  etaSeconds: number;
  assembledFiles: number;
  totalFiles: number;
  scannedFiles: number;
  errorCount: number;
  errorMessage: string;
  warningMessage: string;
  hasDownloadProgress: boolean;
  hasAssemblyProgress: boolean;
}

interface DownloadContextType {
  state: DownloadState;
  setDownloadingGame: (game: Variants | null) => void;
}

const initialState: DownloadState = {
  phase: "idle",
  downloadingGame: null,
  downloadedBytes: 0,
  downloadTotal: 0,
  speedBps: 0,
  etaSeconds: 0,
  assembledFiles: 0,
  totalFiles: 0,
  scannedFiles: 0,
  errorCount: 0,
  errorMessage: "",
  warningMessage: "",
  hasDownloadProgress: false,
  hasAssemblyProgress: false,
};

export const DownloadContext = createContext<DownloadContextType>({
  state: initialState,
  setDownloadingGame: () => {},
});

export const DownloadProvider = ({ children }: { children: ComponentChildren }) => {
  const [state, setState] = useState<DownloadState>({ ...initialState });
  const downloadingGameRef = useRef<Variants | null>(null);

  const setDownloadingGame = (game: Variants | null) => {
    downloadingGameRef.current = game;
    setState((prev) => ({ ...prev, downloadingGame: game }));
  };

  useEffect(() => {
    const unlisten = listen("sophon://progress", (event) => {
      const payload = event.payload as SophonProgress;

      setState((prev) => {
        switch (payload.type) {
          case "fetchingManifest":
            return {
              ...prev,
              phase: "fetchingManifest",
              downloadingGame: prev.downloadingGame ?? downloadingGameRef.current,
              errorMessage: "",
              warningMessage: "",
            };
          case "downloading":
            return {
              ...prev,
              phase: "downloading",
              downloadingGame: prev.downloadingGame ?? downloadingGameRef.current,
              downloadedBytes: payload.downloaded_bytes,
              downloadTotal: payload.total_bytes,
              speedBps: payload.speed_bps,
              etaSeconds: payload.eta_seconds,
              hasDownloadProgress: true,
            };
          case "paused":
            return {
              ...prev,
              phase: "paused",
              downloadedBytes: payload.downloaded_bytes,
              downloadTotal: payload.total_bytes,
            };
          case "assembling":
            return {
              ...prev,
              phase: "assembling",
              assembledFiles: payload.assembled_files,
              totalFiles: payload.total_files,
              hasAssemblyProgress: true,
            };
          case "verifying":
            return {
              ...prev,
              phase: "verifying",
              scannedFiles: payload.scanned_files,
              totalFiles: payload.total_files,
              errorCount: payload.error_count,
            };
          case "warning":
            return {
              ...prev,
              warningMessage: payload.message,
            };
          case "error":
            return {
              ...prev,
              phase: "error",
              errorMessage: payload.message,
            };
          case "finished":
            return {
              ...initialState,
              phase: "finished",
              downloadingGame: null,
            };
        }
      });
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <DownloadContext.Provider value={{ state, setDownloadingGame }}>
      {children}
    </DownloadContext.Provider>
  );
};
