import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "preact/hooks";
import { getOption, setOption } from "../../lib/Settings";
import type { ModalHandle } from "../../types";
import Button from "../Button";
import Modal from "../Modal";

export const UpdateModal = () => {
	const modal = useRef<ModalHandle>(null);
	const [visible, setVisible] = useState<boolean>(false);

	useEffect(() => {
		(async () => {
			const lastUsedVersion = await getOption<string>("lastUsedVersion");
			const currentVersion = await invoke<string>("get_app_version");

			if (currentVersion !== lastUsedVersion) {
				setVisible(true);
				modal.current?.open();
				await setOption<string>("lastUsedVersion", currentVersion);
			}
		})();
	}, []);

	if (!visible) return null;

	return (
		<Modal
			ref={modal}
			closeable={false}
			width={600}
			height={350}
			title="Elysiae Updated!"
		>
			<div class="flex flex-col justify-center">
				<h2>You've Updated Elysiae!</h2>
				<p>
					This is the first release of Elysiae. Everything should run smoothly,
					but there are some minor issues still present in Elysiae
				</p>
				<Button
					variant="primary"
					onClick={async () => {
						modal.current?.close();
					}}
					width={30}
					height={3.25}
					size="md"
				>
					<p>OK</p>
				</Button>
			</div>
		</Modal>
	);
};
