const andList = (names: readonly string[]) =>
  names.length <= 1 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;

/** What a member reads instead of the controls they don't have. */
export const askAdmin = (admins: readonly string[]) =>
  admins.length === 0
    ? 'Only admins can add people or change roles.'
    : `Only admins can add people or change roles. Ask an admin: ${andList(admins)}.`;
