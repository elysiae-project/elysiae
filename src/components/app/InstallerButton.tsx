import { useEffect, useState } from "preact/hooks";
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
import { useDownload } from "../../hooks/useDownload";
import { Pause, Play } from "lucide-preact";

export default function InstallerButton() {
  const { game } = useGame();
  const { state, setDownloadingGame } = useDownload();

  let [wineAvailable, setWineAvailable] = useState<boolean>(false);
  let [gameInstalled, setGameInstalled] = useState<boolean>(false);

  const downloadActive = state.phase !== "idle" && state.phase !== "finished" && state.phase !== "error";
  const downloadPaused = state.phase === "paused";
  const isDownloadForActiveGame = state.downloadingGame === game;

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

  useEffect(() => {
    if (state.phase === "finished" && isDownloadForActiveGame) {
      setGameInstalled(true);
    }
  }, [state.phase, isDownloadForActiveGame]);

  return (
    <div class="w-auto flex flex-row gap-x-3.5">
      {downloadActive && isDownloadForActiveGame ? (
        <Button
          onClick={async () => {
            if (downloadPaused) {
              await resumeDownload();
            } else {
              await pauseDownload();
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
        disabled={downloadActive && !gameInstalled}
        onClick={async () => {
          if (!wineAvailable) {
            await updateWineComponents();
            setWineAvailable(true);
          } else if (!gameInstalled) {
            setDownloadingGame(game);
            await downloadGame(game);
          } else {
            await runGame(game);
          }
        }}>
        {(() => {
          if (!wineAvailable) {
            return "Create Env";
          } else if (!gameInstalled) {
            return downloadActive && isDownloadForActiveGame ? "Downloading..." : "Download";
          } else {
            return "Play";
          }
        })()}
      </Button>
    </div>
  );
}
