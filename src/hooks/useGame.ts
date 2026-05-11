import { GameContext } from "../contexts/GameContext";
import { useContext } from "preact/hooks";

export const useGame = () => {
	const context = useContext(GameContext);
	if (!context) {
		throw new Error("useGame must be used within a GameProvider");
	}
	return context;
};
