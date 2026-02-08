export default function NapModal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	return (
		<>
			{children}
			{title}
		</>
	);
}
