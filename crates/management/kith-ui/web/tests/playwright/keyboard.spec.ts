// The four keyboard-only flows of spec §11.7, each in fewer than 20 keys: approve a join,
// allow a blocked connection, add a phone, set an exit. The fixture server only replays,
// so each change is answered here the way the daemon answers it; reads stay real.

import { copy, DOT } from '../../src/lib/copy';
import { checkCode } from '../../src/lib/format';
import type {
  AllowIn,
  ExitIn,
  PhoneConfig,
  PhoneConfigIn,
  PhoneCreated,
  PhoneIn,
} from '../../src/lib/types.gen';
import {
  accept,
  blockedGroup,
  devices,
  exits,
  expect,
  joinRequest,
  KEY_BUDGET,
  Keys,
  lead,
  network,
  open,
  test,
  thisDevice,
} from './kith';


test('approve a join request', async ({ app }) => {
  const approved: string[] = [];
  await app.route('**/api/networks/*/joins/*/approve', async (route) => {
    approved.push(route.request().url());
    await accept(route);
  });
  const keys = new Keys(app);
  await keys.skipToContent(copy.skipToContent);
  await expect(app.getByText(checkCode(joinRequest.check_code)).first()).toBeVisible();
  await keys.tabTo(app.getByRole('button', { name: copy.join.approve, exact: true }));
  await keys.press('Enter');

  const signed = app.getByText(lead(copy.action.spreading(0, 0)));
  await expect(app.getByText(copy.action.savedHere).or(signed).first()).toBeVisible();
  expect(approved).toEqual([expect.stringContaining(`/joins/${joinRequest.node}/approve`)]);
  expect(keys.count).toBeLessThan(KEY_BUDGET);
});

test('allow a blocked connection, seeing the rule before signing it', async ({ app }) => {
  const previews: boolean[] = [];
  await app.route('**/api/blocked/*/allow', async (route) => {
    const body = route.request().postDataJSON() as AllowIn;
    previews.push(body.preview);
    if (body.preview) await route.continue();
    else await accept(route);
  });
  const { from_name: from, to_name: to, protocol, port } = blockedGroup;
  const keys = new Keys(app);
  await keys.skipToContent(copy.skipToContent);
  await keys.tabTo(app.getByRole('button', { name: copy.blocked.allowThis, exact: true }));
  await keys.press('Enter');

  const rule = copy.blocked.rule(from, to, protocol.toUpperCase(), port ?? 0);
  await expect(app.getByText(rule)).toBeVisible();
  await expect(app.getByText(copy.blocked.nothingElse)).toBeVisible();
  expect(previews, 'the rule is shown before anything is signed').toEqual([true]);
  await keys.tabTo(app.getByRole('button', { name: copy.blocked.addRule, exact: true }));
  await keys.press('Enter');

  await expect(
    app
      .getByText(lead(copy.blocked.spreading(0, 0)))
      .or(app.getByText(copy.blocked.done(from, to, port ?? 0)))
      .or(app.getByText(copy.action.savedHere))
      .first(),
  ).toBeVisible();
  expect(previews).toEqual([true, false]);
  expect(keys.count).toBeLessThan(KEY_BUDGET);
});

test('add a phone from the command palette', async ({ app }) => {
  const taken = devices.some((device) => device.name === copy.addPhone.initial);
  const name = taken ? `${copy.addPhone.initial}-2` : copy.addPhone.initial;
  const created: PhoneIn[] = [];
  const revealed: PhoneConfigIn[] = [];
  await app.route('**/api/networks/*/phones', async (route) => {
    const body = route.request().postDataJSON() as PhoneIn;
    created.push(body);
    const phone: PhoneCreated = {
      name: body.name,
      node: 'a'.repeat(64),
      gateway: thisDevice,
      everything_available: true,
    };
    await route.fulfill({ json: phone });
  });
  await app.route('**/api/networks/*/phones/*/config', async (route) => {
    revealed.push(route.request().postDataJSON() as PhoneConfigIn);
    const config: PhoneConfig = {
      config: '[Interface]\n',
      qr_svg:
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1"/></svg>',
    };
    await route.fulfill({ json: config });
  });
  const keys = new Keys(app);
  await keys.press('ControlOrMeta+k');
  await keys.type(`>${copy.palette.addPhone.split(' ').at(-1) ?? ''}`);
  await keys.press('Enter');

  await expect(app.getByRole('heading', { name: copy.addPhone.title })).toBeVisible();
  await expect(app.getByLabel(copy.addPhone.name, { exact: true })).toHaveValue(name);
  await keys.tabTo(app.getByRole('button', { name: copy.addPhone.create, exact: true }));
  await keys.press('Enter');
  await expect.poll(() => created).toEqual([{ name, tags: [copy.addPhone.tagDefault] }]);

  await keys.tabTo(app.getByRole('button', { name: copy.addPhone.reveal, exact: true }));
  await keys.press('Enter');
  await expect(app.getByText(copy.addPhone.waiting)).toBeVisible();
  expect(revealed).toHaveLength(1);
  expect(keys.count).toBeLessThan(KEY_BUDGET);
});

test('set an exit, which moves at once and confirms when done', async ({ app }) => {
  const asked: ExitIn[] = [];
  let release = (): void => undefined;
  const held = new Promise<void>((resolve) => (release = resolve));
  await app.route('**/api/exit', async (route) => {
    asked.push(route.request().postDataJSON() as ExitIn);
    await held;
    await accept(route);
  });
  await open(app, `/n/${encodeURIComponent(network.name)}/exits`);
  const checked = app.getByRole('radio', { checked: true });
  const before = await checked.ariaSnapshot();
  const keys = new Keys(app);
  await keys.skipToContent(copy.skipToContent);
  await keys.tabTo(checked);
  await keys.press('ArrowDown');

  await expect(app.getByText(copy.exits.switching)).toBeVisible();
  const after = await checked.ariaSnapshot();
  expect(after, 'the choice moves before the daemon answers').not.toBe(before);
  const chosen = exits.find((exit) => after.includes(`"${exit.name}${DOT}`));
  release();

  await expect(
    app.getByText(chosen ? copy.exits.done(chosen.name) : copy.exits.doneNone),
  ).toBeVisible();
  expect(asked).toEqual([expect.objectContaining({ node: chosen?.node ?? null })]);
  expect(keys.count).toBeLessThan(KEY_BUDGET);
});
