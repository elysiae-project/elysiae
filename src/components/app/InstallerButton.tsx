import { useEffect, useRef, useState } from "preact/hooks";
import Button from "../Button";
import { updateWineComponents, wineEnvAvailable } from "../../lib/WineManager";
import {
  downloadGame,
  isGameInstalled,
  pauseDownload,
  resumeDownload,
  runGame,
} from "../../lib/GameDownloader";
import { useGame } from "../../hooks/useGame";
import { SophonProgress, Variants } from "../../types";
import { listen } from "@tauri-apps/api/event";
import { Pause, Play } from "lucide-preact";

export default function InstallerButton() {
  const { game } = useGame();
  const gameRef = useRef(game);
  gameRef.current = game;

  let [wineAvailable, setWineAvailable] = useState<boolean>(false);
  let [gameInstalled, setGameInstalled] = useState<boolean>(false);

  let [currentGameDownload, setCurrentGameDownload] = useState<Variants | null>(
    null,
  );

  let [updatesAvailable, setUpdatesAvailable] = useState<boolean>(false);
  let [downloadInProgress, setDownloadInProgress] = useState<boolean>(false);
  let [downloadPaused, setDownloadPaused] = useState<boolean>(false);

  useEffect(() => {
    const unlisten = listen("sophon://progress", (event) => {
      const payload = event.payload as SophonProgress;
      switch (payload.type) {
        case "fetchingManifest":
        case "downloading":
        case "assembling":
          setDownloadInProgress(true);
          setDownloadPaused(false);
          break;
        case "paused":
          setDownloadInProgress(true);
          setDownloadPaused(true);
          break;
        case "finished":
        case "error":
          setDownloadInProgress(false);
          setDownloadPaused(false);
          setCurrentGameDownload(null);
          if (payload.type === "finished" && currentGameDownload === gameRef.current) {
            setGameInstalled(true);
          }
          break;
        case "warning":
          break;
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    wineEnvAvailable().then((res) => {
      if (!cancelled) setWineAvailable(res);
    });
    isGameInstalled(game).then((res) => {
      if (!cancelled) setGameInstalled(res);
    });
    return () => {
      cancelled = true;
    };
  }, [game]);

  const isDownloadForActiveGame = currentGameDownload === game;

  return (
    <div class="w-auto flex flex-row gap-x-3.5">
      {downloadInProgress && isDownloadForActiveGame ? (
        <Button
          onClick={async () => {
            if (downloadPaused) {
              await resumeDownload();
              setDownloadPaused(false);
            } else {
              await pauseDownload();
              setDownloadPaused(true);
            }
          }}
          intent="secondary"
          iconButton>
          {!downloadPaused ? (
            <Pause className={"leading-0 -m-1"} />
          ) : (
            <Play className={"leading-0 -m-1"} />
          )}
        </Button>
      ) : null}
      <Button
        intent="primary"
        disabled={downloadInProgress && !gameInstalled}
        onClick={async () => {
          if (!wineAvailable) {
            await updateWineComponents();
            setWineAvailable(true);
          } else if (!gameInstalled) {
            const activeGame = game;
            setCurrentGameDownload(activeGame);
            await downloadGame(activeGame);
          } else {
            await runGame(game);
          }
        }}>
        {(() => {
          if (!wineAvailable) {
            return "Create Env";
          } else if (!gameInstalled) {
            return downloadInProgress ? "Downloading..." : "Download";
          } else {
            return "Play";
          }
        })()}
      </Button>
    </div>
  );
}