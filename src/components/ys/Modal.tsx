import { Minimize2 } from "lucide-preact";
import { MutableRef, useEffect, useRef } from "preact/hooks";

export default function YsModal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	const dialog: MutableRef<null | HTMLDialogElement> = useRef(null);

	useEffect(() => {
		if (open) {
			dialog.current?.close();
		} else dialog.current?.showModal();
	}, [open]);

	return (
		<dialog
			ref={dialog}
			class="w-auto min-w-45 h-auto min-h-30 overflow-y-scroll bg-[#3b4354] rounded-lg px-5 py-3"
		>
			<div class="flex flex-row justify-between items-center">
				<h2 class="text-[#d2bc8d]">{title}</h2>
				<Minimize2 color="#d2bc8d" />
			</div>
			<hr class="bg-[#4b5366]"></hr>
			<div>{children}</div>
		</dialog>
	);
}
