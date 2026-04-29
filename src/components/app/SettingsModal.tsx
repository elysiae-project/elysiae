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
				<div class="flex flex-row w-full min-h-full h-auto overflow-y-scroll">
					<div class="min-w-[35%] px-2 py-1.5 h-60 border-r-2 border-gray-500">
						<div class="flex flex-row gap-x-2.5">
							<div class="border-2 rounded-sm">
								<img width={60} height={60} alt="" src={branding?.[game].icon} />
							</div>
							<div class="flex flex-col justify-center">
								<h1 class="text-sm">{getGameName(game)}</h1>
								<h2 class="text-sm">Size On Disk: xxGB</h2>
							</div>
						</div>
					</div>
					<div class="min-w-[65%] px-2 py-1.5">
						
					</div>
				</div>
			</Modal>
		);
	},
);

export default SettingsModal;
