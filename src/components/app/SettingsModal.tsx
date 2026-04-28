import Modal from "../Modal";
import { ModalHandle } from "../../types";
import { forwardRef } from "preact/compat";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import { getGameName } from "../../util/AppFunctions";
import Button from "../Button";

export const SettingsModal = forwardRef<ModalHandle>(
	function SettingsModal(_, ref) {
		const { branding } = useApi();
		const { game } = useGame();

		return (
			<Modal ref={ref} title="Elysiae Settings">
				<h1 class="text-lg">Game Details</h1>
				<div class="flex flex-row gap-x-3">
					<img width={70} height={50} alt="" src={branding?.[game].icon}></img>
					<div class="flex flex-col">
						<h1>{getGameName(game)}</h1>
						<h2>Installed in </h2>
						<h2>Version </h2>
					</div>
				</div>
				<div class="flex flex-row mt-2.5 gap-x-3.5">
					<Button intent="primary" onClick={() => {}}>
						Uninstall
					</Button>
					<Button intent="primary" onClick={() => {}}>
						Check For Updates
					</Button>
				</div>
			</Modal>
		);
	},
);

export default SettingsModal;
