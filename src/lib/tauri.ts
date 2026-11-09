import { invoke } from "@tauri-apps/api/core";

type Confirmation = { id: string; description: string };

export async function confirmedInvoke<T>(
  command: string,
  args: Record<string, unknown>,
  action: string,
  description: string,
): Promise<T | null> {
  const challenge = await invoke<Confirmation>("request_confirmation", { action, description });
  if (!window.confirm(challenge.description)) {
    await invoke("cancel_confirmation", { confirmationId: challenge.id });
    return null;
  }
  const token = await invoke<string>("confirm_action", { confirmationId: challenge.id });
  return invoke<T>(command, { ...args, confirmationToken: token });
}

