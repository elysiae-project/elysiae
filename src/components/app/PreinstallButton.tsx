import { useEffect, useState } from "preact/hooks";
import { useGame } from "../../hooks/useGame";
import { downloadUpdate, isGameInstalled, isPreinstallAvailable } from "../../lib/GameDownloader";
import Button from "../Button";
import { Save } from "lucide-preact";

export default function PreinstallButton() {
	// A lot of placeholder stuff here. Just want to get the component to render so I can implement this stuff in the future
	let [preInstAvailable, setPreInstAvailable] = useState<boolean>(false); // Not implemented yet
	const { game } = useGame();

	useEffect(() => {
		isPreinstallAvailable(game).then((preinstallRes) => {
			isGameInstalled(game).then((gameRes) => {
				setPreInstAvailable(preinstallRes && gameRes);
			});
		});
	}, [game]);

	if (!preInstAvailable) return <></>;

	return (
		<Button
			intent="primary"
			iconButton
			onClick={async () => {
				await downloadUpdate(game, true);
			}}>
			<Save className="leading-0 -m-1" />
		</Button>
	);
}
