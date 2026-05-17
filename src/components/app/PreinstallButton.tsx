import { Save } from "lucide-preact";
import { useEffect, useState } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import {
  downloadUpdate,
  isGameInstalled,
  isPreinstallAvailable,
} from "../../lib/GameDownloader";
import Button from "../Button";

export default function PreinstallButton() {
  const [preInstAvailable, setPreInstAvailable] = useState<boolean>(false);
  const { game } = useGame();
  const { state, setDownloadingGame } = useDownload();
  const downloadActive =
    state.isDownloading ||
    state.isAssembling ||
    state.isVerifying ||
    state.isFetchingManifest ||
    state.isPaused;

  useEffect(() => {
    let cancelled = false;
    isPreinstallAvailable(game).then((preinstallRes) => {
      if (!cancelled) {
        isGameInstalled(game).then((gameRes) => {
          if (!cancelled) setPreInstAvailable(preinstallRes && gameRes);
        });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [game]);

  if (!preInstAvailable) return null;

  return (
    <Button
      variant="secondary"
      width={4}
      height={4}
      disabled={downloadActive}
      onClick={async () => {
        setDownloadingGame(game);
        await downloadUpdate(game, true);
      }}
    >
      <Save className="-m-1 leading-0" />
    </Button>
  );
}
