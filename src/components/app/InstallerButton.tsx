import { useEffect, useState } from "preact/hooks";
import Button from "../Button";
import { updateWineComponents, wineEnvAvailable } from "../../lib/WineManager";
import {
  downloadGame,
  isGameInstalled,
  runGame,
} from "../../lib/GameDownloader";
import { useGame } from "../../hooks/useGame";
import { useDownload } from "../../hooks/useDownload";

export default function InstallerButton() {
  const { game } = useGame();
  const { state, setDownloadingGame } = useDownload();

  let [wineAvailable, setWineAvailable] = useState<boolean>(false);
  let [gameInstalled, setGameInstalled] = useState<boolean>(false);

  const downloadActive = state.isDownloading || state.isAssembling || state.isVerifying || state.isFetchingManifest || state.isPaused;
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
    if (state.isFinished && isDownloadForActiveGame) {
      setGameInstalled(true);
    }
  }, [state.isFinished, isDownloadForActiveGame]);

  return (
    <div class="w-auto flex flex-row gap-x-3.5">
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
