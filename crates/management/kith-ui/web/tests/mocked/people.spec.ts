import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import { ana, cy, expect, network, test } from './console';

const peoplePage = `/n/${network.name}/people`;

test('the owner sees everyone with roles and devices, and adds a member by link', async ({ page, server }) => {
  server.signedIn = ana;
  await page.goto(peoplePage);
  const list = page.getByRole('list', { name: copyConsole.people.title });
  const bo = list.getByRole('listitem').filter({ hasText: 'Bo' });
  await expect(bo).toContainText(copy.account.role.admin);
  await expect(bo).toContainText(copyConsole.people.device('bo-phone', false));
  await expect(list.getByRole('listitem').filter({ hasText: 'Di' })).toContainText(copyConsole.people.pending);

  await page.getByRole('button', { name: copyConsole.people.add }).click();
  const sheet = page.getByRole('dialog', { name: copyConsole.people.add });
  await sheet.getByLabel(copyConsole.people.nameLabel).fill('Eve');
  await sheet.getByRole('button', { name: copyConsole.people.createLink }).click();
  await expect(sheet.getByText(copyConsole.people.linkFor('Eve'))).toBeVisible();
  await expect(sheet.getByText('https://vpn.example.com/welcome/p-eve-token')).toBeVisible();
  expect(server.sent('/api/people').find((c) => c.method === 'POST')?.body).toEqual({ name: 'Eve', role: 'member' });
  expect(server.sent('/api/authority/begin')).toHaveLength(0);
});

test('adding an admin is confirmed with the owner passkey before the link shows', async ({ page, server, authenticator }) => {
  await authenticator.withKey();
  server.signedIn = ana;
  await page.goto(peoplePage);
  await page.getByRole('button', { name: copyConsole.people.add }).click();
  const sheet = page.getByRole('dialog', { name: copyConsole.people.add });
  await sheet.getByLabel(copyConsole.people.nameLabel).fill('Eve');
  await sheet.getByRole('radio', { name: copy.account.role.admin }).click();
  await sheet.getByRole('button', { name: copyConsole.people.createLink }).click();
  await sheet.getByRole('button', { name: copyConsole.people.confirmAdmin }).click();
  await expect(sheet.getByText(copyConsole.people.linkFor('Eve'))).toBeVisible();
  expect(server.sent('/api/authority/begin')[0]?.body).toEqual({ change: { kind: 'add_admin', person: 'p-eve' } });
  expect(server.sent('/api/authority/finish')).toHaveLength(1);
});

test('changing a role says what changes, then the passkey signs it', async ({ page, server, authenticator }) => {
  await authenticator.withKey();
  server.signedIn = ana;
  await page.goto(peoplePage);
  await page.getByRole('button', { name: copyConsole.people.actions('Cy') }).click();
  await page.getByRole('menuitem', { name: copyConsole.people.makeAdmin }).click();

  const dialog = page.getByRole('alertdialog', { name: copyConsole.people.makeAdminTitle('Cy') });
  await expect(dialog).toContainText(copyConsole.people.makeAdminBody('Cy', network.name));
  await dialog.getByRole('button', { name: copyConsole.people.makeAdmin }).click();
  await expect(dialog).toBeHidden();
  expect(server.sent('/api/authority/begin')[0]?.body).toEqual({
    change: { kind: 'set_role', person: cy.id, role: 'admin' },
  });
  expect(server.sent('/api/authority/finish')).toHaveLength(1);

  await page.getByRole('button', { name: copyConsole.people.actions('Bo') }).click();
  await page.getByRole('menuitem', { name: copyConsole.people.remove }).click();
  const remove = page.getByRole('alertdialog', { name: copyConsole.people.removeTitle('Bo') });
  await expect(remove).toContainText(copyConsole.people.removeBody('Bo', 1, network.name));
  await remove.getByRole('button', { name: copyConsole.people.removeConfirm('Bo') }).click();
  await expect(remove).toBeHidden();
  expect(server.sent('/api/authority/begin')[1]?.body).toEqual({ change: { kind: 'remove_person', person: 'p-bo' } });
});

test('members see the list read-only and who to ask', async ({ page, server }) => {
  server.signedIn = cy;
  await page.goto(peoplePage);
  await expect(page.getByText(copyConsole.people.askAdmin(['Ana', 'Bo']))).toBeVisible();
  await expect(page.getByRole('listitem').filter({ hasText: 'Bo' })).toBeVisible();
  await expect(page.getByRole('button', { name: copyConsole.people.add })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Actions for/ })).toHaveCount(0);
  await expect(page.getByRole('button', { name: copy.nav.addDevice })).toHaveCount(0);
});
