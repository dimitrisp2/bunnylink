// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
/**
 * Secrets typed into the interface. JavaScript can't wipe memory, so the interface keeps
 * no copies of its own: password fields are not bound to state, their text is read only
 * when it is sent, and the field is emptied right after.
 */
export type SecretField = HTMLInputElement | HTMLTextAreaElement | undefined | null;

/** The field's text, read for sending. */
export const readSecret = (f: SecretField) => f?.value ?? "";

/** Reads the field's text for sending, and empties the field. */
export function takeSecret(f: SecretField): string {
  const value = f?.value ?? "";
  if (f) f.value = "";
  return value;
}

/** Whether the field has text, without copying it out. The field needs `required`. */
export const hasSecret = (f: SecretField) => !!f && !f.validity.valueMissing;
