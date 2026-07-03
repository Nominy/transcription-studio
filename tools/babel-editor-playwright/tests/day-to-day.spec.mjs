import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

const repoRoot = path.resolve(import.meta.dirname, '..', '..', '..');
const contract = JSON.parse(
  fs.readFileSync(
    path.join(repoRoot, 'crates/studio-app/tests/fixtures/babel-day-to-day-contract.json'),
    'utf8'
  )
);

test.describe('Babel editor day-to-day behavior contract', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveTitle(contract.identity.title);
    await expect(page.getByRole('button', { name: contract.transport.submitLabel })).toBeVisible();
    await expect(page.getByPlaceholder('What was said…').first()).toBeVisible();
    await expect(page.getByText(/Loading Tracks/)).toHaveCount(0, { timeout: 30_000 });
  });

  test('loads the normal transcription editor without review-only controls', async ({ page }) => {
    await expect(page).toHaveURL(new RegExp(`${contract.identity.urlPath.replace('/', '\\/')}$`));

    for (const text of contract.identity.firstMeaningfulTexts) {
      await expect(page.getByText(text, { exact: true }).first()).toBeVisible();
    }

    for (const text of contract.optionalControlsAbsentByDefault) {
      await expect(page.getByText(text, { exact: true })).toHaveCount(0);
    }

    const headerTexts = await page.locator('thead th').evaluateAll((headers) =>
      headers.map((header) => header.textContent?.trim() ?? '')
    );
    expect(headerTexts).toEqual(contract.transcript.headers);

    await expect(page.locator('tbody tr')).toHaveCount(contract.transcript.initialRowCount);
    for (let index = 0; index < contract.transcript.firstRows.length; index += 1) {
      await expect(rowSummary(page, index)).resolves.toMatchObject(
        contract.transcript.firstRows[index]
      );
    }

    await expect(visibleLaneOrder(page)).resolves.toEqual(contract.timeline.laneOrder);
    await expect(defaultRegionStyle(page)).resolves.toMatchObject(
      contract.timeline.defaultVisibleRegion
    );
  });

  test('edits text inline and keeps row actions structural', async ({ page }) => {
    const firstText = page.getByPlaceholder('What was said…').first();
    const original = await firstText.inputValue();
    await firstText.click();
    await firstText.fill(`${original}${contract.transcript.inlineEditDirtySuffix}`);
    await firstText.blur();

    await expect(page.getByRole('button', { name: contract.transport.saveDirtyLabel })).toBeVisible();
    await page.getByRole('button', { name: contract.transport.saveDirtyLabel }).click();
    await expect(page.getByRole('button', { name: contract.transport.saveCleanLabel })).toBeVisible();

    await openFirstRowActionMenu(page);
    for (const item of contract.transcript.rowMenuItems) {
      await expect(page.getByText(item, { exact: true })).toBeVisible();
    }
    for (const item of contract.transcript.rowMenuForbiddenItems) {
      await expect(page.getByText(item, { exact: true })).toHaveCount(0);
    }
  });

  test('copies the support id and exposes the standard transport controls', async ({ page }) => {
    await expect(page.getByText(contract.transport.speedLabel, { exact: true }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: contract.transport.jumpBackLabel })).toBeVisible();
    await expect(page.getByRole('button', { name: contract.transport.browserPlayLabel })).toBeVisible();
    await expect(page.getByRole('button', { name: contract.transport.jumpForwardLabel })).toBeVisible();

    await page.getByRole('button', { name: contract.support.copyLabel }).click();
    await expect(page.getByRole('button', { name: contract.support.copiedLabel })).toBeVisible();
  });

  test('toggles playback through the browser toolbar control', async ({ page }) => {
    await page.getByRole('button', { name: contract.transport.browserPlayLabel }).click();
    await expect(page.getByRole('button', { name: contract.transport.browserPauseLabel })).toBeVisible();

    await page.getByRole('button', { name: contract.transport.browserPauseLabel }).click();
    await expect(page.getByRole('button', { name: contract.transport.browserPlayLabel })).toBeVisible();
  });

  test('opens the recovered Babel hotkeys drawer with daily shortcut sections', async ({ page }) => {
    await page.getByRole('button', { name: contract.hotkeys.buttonLabel }).click();

    await expect(page.getByText(contract.hotkeys.dialogTitle, { exact: true })).toBeVisible();
    const bodyText = await page.locator('body').innerText();
    for (const text of contract.hotkeys.expectedTexts) {
      expect(bodyText).toContain(text);
    }
  });

  test('opens and cancels the normal submit confirmation dialog', async ({ page }) => {
    await page.getByRole('button', { name: contract.transport.submitLabel }).click();

    await expect(page.getByText(contract.submitConfirmation.title, { exact: true })).toBeVisible();
    await expect(page.getByText(contract.submitConfirmation.body, { exact: true })).toBeVisible();
    for (const button of contract.submitConfirmation.buttons) {
      await expect(page.getByRole('button', { name: button })).toBeVisible();
    }

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByText(contract.submitConfirmation.title, { exact: true })).toHaveCount(0);
  });

  test('dragging empty waveform space immediately creates a blank segment and cancel only dismisses', async ({
    page
  }) => {
    const originalCount = await page.locator('tbody tr').count();

    await dragCreateRegionOnSpeaker1(page);

    await expect(page.getByText(contract.createRegion.dialogTitle, { exact: true })).toBeVisible();
    await expect(page.getByPlaceholder(contract.createRegion.textPlaceholder)).toBeVisible();
    await expect(page.locator('tbody tr')).toHaveCount(
      originalCount + contract.createRegion.createdSegmentCountDeltaOnDrag
    );
    await expect(rowSummary(page, 0)).resolves.toMatchObject({
      displayId: '01',
      speaker: contract.createRegion.laneLabel,
      text: ''
    });
    await expect(page.getByRole('button', { name: contract.transport.saveDirtyLabel })).toBeVisible();

    const cardText = await createRegionCardText(page);
    for (const field of contract.createRegion.forbiddenFields) {
      expect(cardText).not.toContain(field);
    }

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByText(contract.createRegion.dialogTitle, { exact: true })).toHaveCount(0);
    await expect(page.locator('tbody tr')).toHaveCount(
      originalCount + contract.createRegion.createdSegmentCountDeltaOnDrag
    );
    await expect(rowSummary(page, 0)).resolves.toMatchObject({
      displayId: '01',
      speaker: contract.createRegion.laneLabel,
      text: ''
    });
  });

  test('saving the Add Region card updates the segment created by the drag', async ({ page }) => {
    const originalCount = await page.locator('tbody tr').count();

    await dragCreateRegionOnSpeaker1(page);
    await page.getByPlaceholder(contract.createRegion.textPlaceholder).fill(contract.createRegion.savedText);
    await page.getByRole('button', { name: 'Save', exact: true }).click();

    await expect(page.getByText(contract.createRegion.dialogTitle, { exact: true })).toHaveCount(0);
    await expect(page.locator('tbody tr')).toHaveCount(
      originalCount + contract.createRegion.createdSegmentCountDeltaOnDrag
    );
    await expect(rowSummary(page, 0)).resolves.toMatchObject({
      displayId: '01',
      speaker: contract.createRegion.laneLabel,
      text: contract.createRegion.savedText
    });
    await expect(page.getByRole('button', { name: contract.transport.saveDirtyLabel })).toBeVisible();
  });

  test('clicking existing regions selects inline transcript editing instead of opening a text popup', async ({
    page
  }) => {
    const region = await defaultRegionBox(page);
    await page.mouse.click(region.x + Math.min(80, region.width / 2), region.y + region.height / 2);

    await expect(page.getByText(contract.createRegion.dialogTitle, { exact: true })).toHaveCount(0);
    await expect(page.getByPlaceholder(contract.createRegion.textPlaceholder)).toHaveCount(0);

    await page.mouse.dblclick(region.x + Math.min(90, region.width / 2), region.y + region.height / 2);
    await expect(page.getByText(contract.createRegion.dialogTitle, { exact: true })).toHaveCount(0);
    await expect(page.getByPlaceholder(contract.createRegion.textPlaceholder)).toHaveCount(0);
  });
});

async function rowSummary(page, index) {
  return page.locator('tbody tr').nth(index).evaluate((row) => {
    const cells = [...row.querySelectorAll('td')];
    const textBox = cells[4]?.querySelector('textarea,input,[contenteditable="true"]');
    const text =
      textBox instanceof HTMLTextAreaElement || textBox instanceof HTMLInputElement
        ? textBox.value
        : textBox?.textContent?.trim() ?? cells[4]?.textContent?.trim() ?? '';
    return {
      displayId: cells[0]?.textContent?.trim(),
      speaker: cells[1]?.textContent?.trim(),
      start: cells[2]?.textContent?.trim(),
      end: cells[3]?.textContent?.trim(),
      text,
      textPrefix: text.slice(0, 9)
    };
  });
}

async function visibleLaneOrder(page) {
  return page.locator('h3').evaluateAll((headings) =>
    headings
      .map((heading) => ({
        text: heading.textContent?.trim() ?? '',
        y: heading.getBoundingClientRect().top
      }))
      .filter((heading) => heading.text.startsWith('Speaker '))
      .sort((a, b) => a.y - b.y)
      .map((heading) => heading.text)
  );
}

async function defaultRegionStyle(page) {
  return page.evaluate(({ expected }) => {
    function collectElements(root) {
      const elements = [...root.querySelectorAll('*')];
      for (const element of [...elements]) {
        if (element.shadowRoot) {
          elements.push(...collectElements(element.shadowRoot));
        }
      }
      return elements;
    }

    function averageSpeakerHeadingY(label) {
      const heading = [...document.querySelectorAll('h3')].find(
        (candidate) => candidate.textContent?.trim() === label
      );
      return heading?.getBoundingClientRect().top ?? 0;
    }

    const regions = collectElements(document)
      .map((element) => {
        const style = getComputedStyle(element);
        const box = element.getBoundingClientRect();
        return {
          element,
          box,
          backgroundColor: style.backgroundColor,
          borderColor: style.borderRightColor
        };
      })
      .filter(
        (region) =>
          region.box.width > 20 &&
          region.box.height > 10 &&
          region.backgroundColor === expected.backgroundColor &&
          region.borderColor === expected.borderColor
      );
    if (regions.length === 0) {
      throw new Error('No default visible region with the recovered Babel color was found');
    }
    const region = regions.sort((a, b) => b.box.width * b.box.height - a.box.width * a.box.height)[0];
    const laneLabel =
      region.box.top > averageSpeakerHeadingY('Speaker 1') ? 'Speaker 2' : 'Speaker 1';
    return {
      laneLabel,
      backgroundColor: region.backgroundColor,
      borderColor: region.borderColor
    };
  }, { expected: contract.timeline.defaultVisibleRegion });
}

async function defaultRegionBox(page) {
  return page.evaluate(({ expected }) => {
    function collectElements(root) {
      const elements = [...root.querySelectorAll('*')];
      for (const element of [...elements]) {
        if (element.shadowRoot) {
          elements.push(...collectElements(element.shadowRoot));
        }
      }
      return elements;
    }

    const regions = collectElements(document)
      .map((element) => {
        const style = getComputedStyle(element);
        const box = element.getBoundingClientRect();
        return {
          x: box.left,
          y: box.top,
          width: box.width,
          height: box.height,
          area: box.width * box.height,
          backgroundColor: style.backgroundColor,
          borderColor: style.borderRightColor
        };
      })
      .filter(
        (region) =>
          region.width > 20 &&
          region.height > 10 &&
          region.backgroundColor === expected.backgroundColor &&
          region.borderColor === expected.borderColor
      )
      .sort((a, b) => b.area - a.area);
    if (regions.length === 0) {
      throw new Error('No clickable existing region was found');
    }
    return regions[0];
  }, { expected: contract.timeline.defaultVisibleRegion });
}

async function dragCreateRegionOnSpeaker1(page) {
  const drag = await page.evaluate(() => {
    function collectElements(root) {
      const elements = [...root.querySelectorAll('*')];
      for (const element of [...elements]) {
        if (element.shadowRoot) {
          elements.push(...collectElements(element.shadowRoot));
        }
      }
      return elements;
    }

    const canvases = collectElements(document)
      .filter((element) => element.tagName === 'CANVAS')
      .map((canvas) => {
        const box = canvas.getBoundingClientRect();
        return {
          x: box.left,
          y: box.top,
          width: box.width,
          height: box.height
        };
      })
      .filter((box) => box.width > 200 && box.height > 100)
      .sort((a, b) => a.y - b.y || a.x - b.x);
    if (canvases.length === 0) {
      throw new Error('Speaker 1 waveform canvas was not found');
    }
    const box = canvases[0];
    return {
      x1: box.x + 60,
      x2: box.x + 230,
      y: box.y + box.height / 2
    };
  });
  await page.mouse.move(drag.x1, drag.y);
  await page.mouse.down();
  await page.mouse.move(drag.x2, drag.y, { steps: 8 });
  await page.mouse.up();
}

async function openFirstRowActionMenu(page) {
  const actionButton = page.locator('tbody tr').first().locator('button');
  await expect(actionButton).toHaveCount(1);
  await actionButton.click();
}

async function createRegionCardText(page) {
  return page.getByPlaceholder(contract.createRegion.textPlaceholder).evaluate((input) => {
    let node = input.parentElement;
    while (
      node?.parentElement &&
      !(
        node.textContent?.includes('Add Region') &&
        node.textContent.includes('Cancel') &&
        node.textContent.includes('Save')
      )
    ) {
      node = node.parentElement;
    }
    return node?.textContent ?? '';
  });
}
