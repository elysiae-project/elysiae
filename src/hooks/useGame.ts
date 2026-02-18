import { useContext } from "preact/hooks";
import { GameContext } from "../contexts/GameContext";

export const useGame = () => {
	const context = useContext(GameContext);
	if (!context) {
		throw new Error("useGame must be used within a GameProvider");
	}
	return context;
};
