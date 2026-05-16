import { getActiveGameCode, getVariantFromCode } from "../util/AppFunctions";
import { createContext, ComponentChildren } from "preact";
import { getOption, setOption } from "../util/Settings";
import { useEffect, useState } from "preact/hooks";
import { warn } from "@tauri-apps/plugin-log";
import { Variants } from "../types";

interface GameContextType {
	game: Variants;
	setGame: (variant: Variants) => void;
}

export const GameContext = createContext<GameContextType>({
	game: Variants.NAP,
	setGame: () => {},
});

export const GameProvider = ({ children }: { children: ComponentChildren }) => {
	const [game, setGame] = useState<Variants>(Variants.HKRPG);
	useEffect(() => {
		(async () => {
			const lastSelectedGame = getVariantFromCode(
				await getOption<string>("selectedGame"),
			);
			if (lastSelectedGame) {
				setGame(lastSelectedGame);
			} else {
				warn("GameProvider: selectedGame option is missing, null, or empty");
			}
		})();
	}, []);

	useEffect(() => {
		(async () => {
			const gameCode = getActiveGameCode(game);
			await setOption("selectedGame", gameCode);
		})();
	}, [game]);

	return (
		<GameContext.Provider value={{ game, setGame }}>
			{children}
		</GameContext.Provider>
	);
};
