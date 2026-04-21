import { useDownload } from "../../hooks/useDownload";
import { getGameName, formatNumber } from "../../util/AppFunctions";
import Progressbar from "../Progressbar";

export default function GameDownloadProgress() {
  const { state } = useDownload();

  const isActive = state.phase !== "idle" && state.phase !== "finished";
  if (!isActive) return null;

  const downloadPct = state.downloadTotal > 0 ? (state.downloadedBytes / state.downloadTotal) * 100 : 0;
  const assemblePct = state.totalFiles > 0 ? (state.assembledFiles / state.totalFiles) * 100 : 0;
  const speedMB = state.speedBps / 1024 ** 2;
  const eta = state.etaSeconds;
  const etaStr = eta > 0
    ? eta >= 3600
      ? `${Math.floor(eta / 3600)}h ${Math.floor((eta % 3600) / 60)}m`
      : `${Math.floor(eta / 60)}m ${Math.floor(eta % 60)}s`
    : "";

  const titleText = state.phase === "paused"
    ? "Download Paused"
    : state.phase === "verifying"
    ? "Verifying Files..."
    : state.phase === "fetchingManifest"
    ? "Fetching Manifest..."
    : state.downloadingGame !== null
    ? `Downloading ${getGameName(state.downloadingGame)}...`
    : "Downloading...";

  return (
    <div class="flex flex-col w-full h-auto gap-y-3 justify-start items-start align-bottom text-white bg-black/50 rounded-lg mr-10 px-4 py-5">
      <h1 class="-mt-2 mb-0.5">{titleText}</h1>
      {state.hasDownloadProgress && (
        <div class="flex flex-col min-w-full text-left gap-y-1">
          <h2 class="text-sm ml-1">
            Downloaded {(state.downloadedBytes / 1024 ** 3).toFixed(2)}GB of{" "}
            {(state.downloadTotal / 1024 ** 3).toFixed(2)}GB ({downloadPct.toFixed(2)}%)
            {speedMB > 0 ? ` - ${speedMB.toFixed(2)}MB/s` : ""}
            {etaStr ? ` - ETA: ${etaStr}` : ""}
          </h2>
          <Progressbar progress={downloadPct} />
        </div>
      )}
      {state.hasAssemblyProgress && (
        <div class="flex min-w-full flex-col text-left gap-y-1">
          <h2 class="text-sm ml-1">
            Assembled {formatNumber(state.assembledFiles)} of {formatNumber(state.totalFiles)} Files ({assemblePct.toFixed(2)}%)
          </h2>
          <Progressbar progress={assemblePct} />
        </div>
      )}
      {state.phase === "verifying" && (
        <div class="flex min-w-full flex-col text-left gap-y-1">
          <h2 class="text-sm ml-1">
            Verified {formatNumber(state.scannedFiles)} of {formatNumber(state.totalFiles)} files — {formatNumber(state.errorCount)} errors found
          </h2>
          <Progressbar progress={state.totalFiles > 0 ? (state.scannedFiles / state.totalFiles) * 100 : 0} />
        </div>
      )}
      {state.warningMessage && (
        <h2 class="text-sm ml-1 text-yellow-300">{state.warningMessage}</h2>
      )}
      {state.errorMessage && (
        <h2 class="text-sm ml-1 text-red-300">{state.errorMessage}</h2>
      )}
    </div>
  );
}
