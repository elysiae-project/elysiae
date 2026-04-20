import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "preact/hooks";
import { SophonProgress } from "../../types";
import Progressbar from "../Progressbar";
import { formatNumber } from "../../util/AppFunctions";
import { useGame } from "../../hooks/useGame";

export default function GameDownloadProgress() {
  let [downloadedBytes, setDownloadedBytes] = useState<number>(0);
  let [downloadTotal, setDownloadTotal] = useState<number>(0);
  let [assembledFiles, setAssembledFiles] = useState<number>(0);
  let [totalFiles, setTotalFiles] = useState<number>(0);
  let [isDownloading, setIsDownloading] = useState<boolean>(false);
  let [isPaused, setIsPaused] = useState<boolean>(false);

  const { game } = useGame();
  const activeGameRef = useRef(game);
  activeGameRef.current = game;

  useEffect(() => {
    const unlisten = listen("sophon://progress", (event) => {
      const payload = event.payload as SophonProgress;
      switch (payload.type) {
        case "fetchingManifest":
          setIsDownloading(true);
          setIsPaused(false);
          break;
        case "downloading":
          setIsDownloading(true);
          setIsPaused(false);
          setDownloadedBytes(payload.downloaded_bytes);
          setDownloadTotal(payload.total_bytes);
          break;
        case "paused":
          setIsDownloading(true);
          setIsPaused(true);
          break;
        case "assembling":
          setIsDownloading(true);
          setIsPaused(false);
          setAssembledFiles(payload.assembled_files);
          setTotalFiles(payload.total_files);
          break;
        case "verifying":
          setIsDownloading(true);
          setIsPaused(false);
          break;
        case "finished":
          setIsDownloading(false);
          setIsPaused(false);
          setDownloadedBytes(0);
          setDownloadTotal(0);
          setAssembledFiles(0);
          setTotalFiles(0);
          break;
        case "error":
          setIsDownloading(false);
          setIsPaused(false);
          setDownloadedBytes(0);
          setDownloadTotal(0);
          setAssembledFiles(0);
          setTotalFiles(0);
          break;
        case "warning":
          break;
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  if (!isDownloading) return null;

  const downloadPct = downloadTotal > 0 ? (downloadedBytes / downloadTotal) * 100 : 0;
  const assemblePct = totalFiles > 0 ? (assembledFiles / totalFiles) * 100 : 0;

  return (
    <div class="flex flex-col w-full h-auto gap-y-3 justify-start items-start align-bottom text-white bg-black/50 rounded-lg mr-10 px-4 py-5">
      <h1 class="-mt-2 mb-0.5">
        {isPaused ? "Download Paused" : "Downloading..."}
      </h1>
      <div class="flex flex-col min-w-full text-left gap-y-1">
        <h2 class="text-sm ml-1">
          Downloaded {(downloadedBytes / 1024 ** 3).toFixed(2)}GB of{" "}
          {(downloadTotal / 1024 ** 3).toFixed(2)}GB ({downloadPct.toFixed(2)}%)
        </h2>
        <Progressbar progress={downloadPct} />
      </div>
      <div class="flex min-w-full flex-col text-left gap-y-1">
        <h2 class="text-sm ml-1">
          Assembled {formatNumber(assembledFiles)} of {formatNumber(totalFiles)} Files ({assemblePct.toFixed(2)}%)
        </h2>
        <Progressbar progress={assemblePct} />
      </div>
    </div>
  );
}