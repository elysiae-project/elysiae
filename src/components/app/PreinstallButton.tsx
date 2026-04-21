import { useEffect, useState } from "preact/hooks";
import { useGame } from "../../hooks/useGame";
import { downloadUpdate, isGameInstalled, isPreinstallAvailable } from "../../lib/GameDownloader";
import { useDownload } from "../../hooks/useDownload";
import Button from "../Button";
import { Save } from "lucide-preact";

export default function PreinstallButton() {
  let [preInstAvailable, setPreInstAvailable] = useState<boolean>(false);
  const { game } = useGame();
  const { state, setDownloadingGame } = useDownload();

  const downloadActive = state.isDownloading || state.isAssembling || state.isVerifying || state.isFetchingManifest || state.isPaused;

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

  if (!preInstAvailable) return <></>;

  return (
    <Button
      intent="primary"
      iconButton
      disabled={downloadActive}
      onClick={async () => {
        setDownloadingGame(game);
        await downloadUpdate(game, true);
      }}>
      <Save className="leading-0 -m-1" />
    </Button>
  );
}
