import Modal from "../Modal";
import { ModalHandle } from "../../types";
import { forwardRef } from "preact/compat";

export const SettingsModal = forwardRef<ModalHandle>(
	function SettingsModal(_, ref) {
		return (
			<Modal ref={ref} title="Settings">
				<p>TEST</p>
			</Modal>
		);
	},
);

export default SettingsModal;
