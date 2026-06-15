import { useEffect, useRef, useState } from "preact/compat";
import { getOption, setOption } from "../../lib/Settings";
import type { ModalHandle } from "../../types";
import Button from "../Button";
import Modal from "../Modal";

export const PhotosensitivityModal = () => {
	const modal = useRef<ModalHandle>(null);
	const [visible, setVisible] = useState<boolean>(false);

	useEffect(() => {
		(async () => {
			const firstLaunch = (await getOption<boolean>("isFirstLaunch")) ?? false;
			setVisible(firstLaunch);
			if (firstLaunch) {
				modal.current?.open();
			}
		})();
	}, []);

	if (!visible) return null;

	return (
		<Modal ref={modal} width={550} height={250} closeable={false}>
			<div class="flex flex-col justify-center gap-y-3 pb-3">
				<h1 class="text-2xl">Warning</h1>
				<p class="text-left">
					Elysiae depends on{" "}
					<a href="https://winehq.org" class="underline">
						Proton
					</a>
					to run games on Linux. Due to the nature of the tools Proton uses,
					applications running through Proton are more prone to rendering errors
					that can trigger seizures in certain individuals. If you are sensitive
					to flashing lights, please take caution when using Elysiae and other
					applications that use Proton. While these issues are extremely rare,
					it is best to stay cautious
				</p>
				<div class="flex justify-center">
					<Button
						variant="primary"
						onClick={async () => {
							await setOption("isFirstLaunch", false);
							modal.current?.close();
						}}
						width={30}
						height={3.25}
						size="md"
					>
						<p>OK</p>
					</Button>
				</div>
			</div>
		</Modal>
	);
};

export default PhotosensitivityModal;
