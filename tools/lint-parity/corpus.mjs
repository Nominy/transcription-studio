// Lint parity corpus: transcripts that exercise every gold-non-bg rule and every Helper rule,
// each with passing and failing text. Segments are { id, text, start, end, track }.

function single(language, text, name = text) {
  return { name: `${language}: ${name}`, language, segments: [{ id: 's1', text, start: 1, end: 3, track: 't1' }] };
}

function transcript(language, name, segments) {
  return {
    name: `${language}: ${name}`,
    language,
    segments: segments.map(([text, start, end, track = 't1'], index) => ({ id: `s${index + 1}`, text, start, end, track })),
  };
}

const RU_TEXTS = [
  // Clean text and common shapes.
  'Привет. Как дела?',
  'Да, конечно.',
  'Ну... Я не знаю.',
  'Слушай, а вот какая у тебя есть привычка?',
  'Он сказал: "Приходи завтра".',
  '...и потом мы ушли.',
  'Я думал-- Нет, не важно.',
  'Я ду- думал об этом.',
  'Это к- кот.',
  'Ни- ник- никто не пришёл.',
  '5 {СКАЗ: пять} человек.',
  '[смех] Да, точно.',
  'Да [смех] нет.',
  '<смех-в-речи> Да, точно. </смех-в-речи>',
  '<иностранная-речь> hello </иностранная-речь> мир.',
  '<иностранная-речь="EN"> hello </иностранная-речь> мир.',
  '[неразборчиво]',
  'Это [неразборчиво]',
  // Empty and whitespace.
  '',
  '   ',
  'Да  нет.',
  ' Да.',
  'Да. ',
  'Да\nнет.',
  'Да\r\nнет.',
  // Emphasis.
  'Это **важно**.',
  'Это *важно.',
  'Это * * важно.',
  'Это *важно.* да.',
  'Это *важно* да.',
  'Это * важно * да.',
  'Это *важно * да.',
  'Это * важно* да.',
  // Square brackets.
  '[смехх] Да.',
  '[ смех ] Да.',
  '[] Да.',
  '[смех Да.',
  'Да] нет.',
  '[иностранная-речь] да.',
  '[другой-звук] да.',
  '[музыка] да.',
  '[Смех] да.',
  // Style tags.
  '<плохо> да </плохо>',
  '< смех-в-речи> да </смех-в-речи>',
  '<> да.',
  '<смех-в-речи> да.',
  '<смех-в-речи></смех-в-речи> да.',
  '<смех-в-речи>да</смех-в-речи>',
  '<радостно> Да. </радостно>',
  '<радостно> Да </радостно>.',
  '<радостно> Да </радостно> .',
  'да > нет.',
  'да <радостно> нет </грустно>.',
  '<иностранная-речь="XX"> hello </иностранная-речь>',
  // Curly brackets.
  '{ } да.',
  '{СКАЗ: пять',
  'да}.',
  '{СКАЗ:пять} да.',
  '5 {СКАЗ:пять} да.',
  '{СКАЗ : пять} да.',
  '5{СКАЗ: пять} да.',
  '{пять} да.',
  '{{комментарий}} да.',
  '5 {ИСКАЖ: слово} да.',
  '5 {PRO: five} да.',
  '5 {СКАЗ: 5} да.',
  '12 {PRO: 12} да.',
  'СКАЗ: пять.',
  'ИСКАЖ: слово.',
  'неразборчиво да.',
  'MIS: x.',
  '{СКАЗ: пять}{ИСКАЖ: слово}',
  // Non-speech tag spacing.
  'Да[смех] нет.',
  'Да [смех]нет.',
  'Да [смех], нет.',
  '"[смех]" да.',
  'Да [смех]-- нет.',
  // Dashes.
  'Да --- нет.',
  'Да -- нет.',
  'Да--. Нет.',
  'Да--нет.',
  'Да--',
  'Да--" нет.',
  'Да--- нет.',
  'Да — нет.',
  'Да – нет.',
  'Да—нет.',
  'Да -нет.',
  'Да- нет.',
  'Да -, нет.',
  'Да --, нет.',
  'Да--... нет.',
  'Кто-то пришёл.',
  'Мы шли -- и шли.',
  // Punctuation runs.
  'Да!! Нет.',
  'Да?! Нет.',
  'Да,, нет.',
  'Да... Нет.',
  'Да.. Нет.',
  'Да?.. Нет.',
  'Да!? Нет!',
  'Привет,мир.',
  'Привет ,мир.',
  'Число 3,14.',
  'Готово.Дальше.',
  'Готово. Дальше.',
  'Да ,нет.',
  // Quotes.
  'Он сказал "да.',
  'Он сказал «да».',
  'Он сказал “да”.',
  'Рост 5\'10" примерно.',
  'Он сказал "да" и "нет".',
  // Parentheses, characters, emoji.
  'Да (нет).',
  'a = b.',
  'x -> y.',
  '2 × 3.',
  'Да 😀.',
  'Да ❤️.',
  'Да ™.',
  // Repetition heuristics.
  'ПРИВЕТ мир.',
  'Привет МИР.',
  'Дааааа.',
  'Ааааа.',
  '1234567890123.',
  '123456789012.',
  'Номер 11111.',
  // Tag-only segments.
  '[смех].',
  '[смех]',
  '[смех] [кашель]',
  '<смех-в-речи> [смех] </смех-в-речи>',
  '{{комментарий}}.',
  // Capitalization and pronouns.
  'начало.',
  'Готово. дальше.',
  'Готово! дальше.',
  'Готово? дальше.',
  'Как дела? - спросил он.',
  '"Как дела?" - спросил он.',
  'Я сказал Вам.',
  'Вы пришли.',
  'Я сказал: Вы правы.',
  'Я видел Ваш дом.',
  '<радостно> Вы </радостно> пришли.',
  // Terminal punctuation.
  'Конец',
  'Конец [смех]',
  'Конец?',
  'Конец!',
  'Конец...',
  'Конец--',
  'Конец -',
  'Конец "',
  'Конец, [неразборчиво]',
  'Он сказал «конец»',
  // Interjections, ё, highlighted words.
  'Ммм, понятно.',
  'Ааа, понятно.',
  'Эээ, понятно.',
  'Угу, понятно.',
  'Всё, угу.',
  'Все пришли.',
  'Ещё ёлка. Всё обо всём, берёт, берёте, о нём.',
  'Её нет.',
  'Мёд вкусный.',
  'Он берёт.',
  // Stutters.
  'нь- нет.',
  'Нь- нет.',
  'К- никто.',
  'Что-же- что-то.',
  'А- а- один.',
  // Tag punctuation and spacing (Helper).
  'Да<TAG>нет</TAG>.',
  'Да <TAG> нет. </TAG>',
  'Да </TAG>.',
  'Да. </TAG>',
  '3 {SKAZ: три}.',
  '3{SKAZ: три}.',
  '3. {SKAZ: три}',
  'Да [смех].',
  'Да. [смех]',
  'Да[смех]нет.',
  'Да -.',
  'Да - нет.',
  'Да--, нет.',
  // Long and repeated text.
  'Очень длинное предложение без повторов, которое просто продолжается дальше и дальше, пока не станет длиннее ста символов вообще.',
  'Это повторяющийся фрагмент текста ровно на пятьдесят знаков! Это повторяющийся фрагмент текста ровно на пятьдесят знаков!',
];

// Branches of the Helper custom-linter rules (failing and passing), one line per shape.
const RU_HELPER_TEXTS = [
  // Leading/trailing and double spaces.
  '\tДа.\t',
  'Да \u00a0 нет.',
  '  Да  нет.',
  'Да.  ',
  'Да\u00a0\u00a0нет.',
  '😀  да.',
  // Commas.
  'Да,\nнет.',
  'Да,  нет.',
  'Да,',
  'Да,"нет".',
  'Да\u00a0, нет.',
  'Счёт 1,5 и 2, 3.',
  // Periods.
  'Да .Нет.',
  'Да.  Нет.',
  'Да.<радостно> нет </радостно>',
  'Версия 1.5 вышла.',
  '{СКАЗ: т.е.} да.',
  'Да.\tНет.',
  'Да.[смех]',
  'Да. "Нет".',
  'Да."Нет".',
  'Да.{СКАЗ: да}',
  'Да.😀',
  // Angle and square tag spacing.
  '<радостно >да</радостно>',
  'Да <радостно> нет </ радостно>.',
  'Да <радостно> нет < /радостно>.',
  'Да\u00a0<радостно> нет </радостно>.',
  'Да😀[смех] нет.',
  'Да [ /смех] нет.',
  // Curly tag spacing.
  'Да{СКАЗ: да}.',
  'Да {СКАЗ: да}.',
  '😀{СКАЗ: да}.',
  'Да{пять}.',
  'Да{СКАЗ да: x}.',
  'Да{СКАЗ:}.',
  'Да{СКАЗ: да\nнет}.',
  // Quote balance and Unicode quotes/dashes.
  'Он сказал «да.',
  '«Да» и "нет".',
  '„Да“.',
  'Да ‟ нет ”.',
  'Да‐нет.',
  'Да−нет.',
  'Да ― нет.',
  'Да ⸺ нет.',
  // Punctuation after curly tags.
  '5 {СКАЗ: пять}, да.',
  '5 {СКАЗ: пять} "да".',
  '{СКАЗ: пять}, да.',
  '5 {СКАЗ: пять}...да.',
  '5 {СКАЗ: пять} ... да.',
  '5 {СКАЗ: пять}"',
  '5 {СКАЗ: пять} ;: да.',
  '[смех] {СКАЗ: пять}, да.',
  // Punctuation after/before style tags.
  '<радостно> Да </радостно>, нет.',
  'Он сказал "<радостно> да </радостно>".',
  'Он " <радостно> да. </радостно>',
  'Он"<радостно> да. </радостно>',
  'Он "" <радостно> да. </радостно>',
  '" <радостно> Да. </радостно>',
  'Он " <радостно> </радостно>',
  '<радостно> </радостно>, да.',
  'Да <радостно> нет </радостно> "да".',
  // Punctuation after square-bracket tags.
  'Да [смех], нет.',
  'Да [смех]- нет.',
  '[смех], да.',
  'Да [смех]...нет.',
  'Да [смех] ... нет.',
  'Да [смех] "нет".',
  'Да [смех] [кашель].',
  // Free double dash.
  'Он сказал "мы -- да".',
  '-- Да.',
  'Да --',
  'Да  --  нет.',
  '{да -- нет} да.',
  'Да\t--\tнет.',
  'Да -- -- нет.',
  // Punctuation after dashes.
  '[да--, нет] да.',
  'Да--?! Нет.',
  'Да--.. нет.',
  'Да-, нет.',
  'Да -. Нет.',
  '<тег-.> да.',
  'Да---, нет.',
  // Only the first eight matches survive compaction before the in-tag filter.
  `${'[да--,] '.repeat(8)}Да--, нет.`,
  `${'[да-,] '.repeat(7)}Да-, нет.`,
  // Terminal punctuation.
  'Конец [неразборчиво]  ',
  'Конец {СКАЗ: да}',
  'Конец <радостно>',
  'Конец»',
  'Конец;',
  'Конец:',
  'Конец, [смех] [кашель]',
  'Конец [смех',
  'Конец 😀',
  '[смех]   ',
  // Stutters.
  'К- к- кот.',
  'Ко- кот- кошка.',
  'Дом- до.',
  'Ко- 5 кот.',
  'Ко-ко- кошка.',
  'Ко- ко- [смех] кот.',
  'Ко- Кот.',
  'Ко-  кот.',
  'Ко- -кот.',
  'Кот- ко- кот.',
  'Я ко- кот- кошка.',
  'Шо- 5шоколад.',
  // Interjections.
  'Мда, ладно.',
  'Ну, да.',
  'О\'кей.',
  'Ей богу.',
  'Э…э, нет.',
  'Хаха!',
  'МДЯ, ладно.',
  'Мм-гм.',
  'Ааронов пришёл.',
  'Ммммм.',
  'А-а-ах, вот.',
  'Нууу_ да.',
  // Unnecessary ё.
  'Зелёный лес.',
  'Ёлка зелёная.',
  'Ёлка.',
  'Е\u0308лка е\u0308лка.',
  'Чёрно‐белый, чёрно-белый.',
  'Сказал «зелёный».',
  '[зелёный] да.',
  'По-моему, ещё.',
  'Всё-таки вёл.',
  'Ещё-то.',
  'Нёс.',
  'НЁС.',
  // Highlighted words.
  'Всё хорошо.',
  'Он берет.',
  'Мм, м-м, да.',
  '[все] да.',
  'Всем привет.',
  'Всеми силами.',
  'ВСЁ.',
  'О нём, о нем.',
  'все_ да.',
  `${'[все] '.repeat(8)}все.`,
  `${'[все] '.repeat(7)}все.`,
  // Sentence boundaries.
  'Да. нет.',
  'Да?! нет.',
  '"Да!" сказал он.',
  'Он сказал: "Нет?" - и ушёл.',
  '"Нет!" - Сказал он.',
  '"Нет?!" -- сказал он.',
  'Да. [смех] нет.',
  'Да. 5 человек.',
  'Т.е. да.',
  'Да. (нет) да.',
  'Да? «нет».',
  'Да! {СКАЗ: да} нет.',
  'Да. <радостно> нет </радостно>',
  '"Да? - спросил он.',
  'Да? -спросил он.',
  // Polite pronouns.
  'Как Вы?',
  'Скажите: Вы правы.',
  'Да - Вы правы.',
  '"Вы" пришли.',
  '[смех] Вы правы.',
  'Да. "Вы правы".',
  'Я <радостно> Вас </радостно> жду.',
  '{Вы} да.',
  'Да (Вы) нет.',
  'Я ВАШЕМУ брату.',
  'Я вам скажу.',
  'Я Вами доволен.',
  'Я Вашими силами.',
  'Да </радостно> Вы.',
  'Да [смех] Вы.',
  'Да? [смех] Вы.',
  'Привет, ВЫ.',
  'ВыВы да.',
  // Segment start.
  '[смех] начало.',
  '5 человек.',
  '{СКАЗ: пять} человек.',
  '... начало.',
  '[смех] ...начало.',
  '"начало".',
  '<радостно> начало </радостно>',
  '[смех начало.',
  // Non-BMP characters around indexes.
  '😀 начало. 😀 конец',
  '𝐀бв. где.',
];

// Gold (Babel native linter) branches beyond Russian: per-language keywords, bracket types and
// punctuation tables, CJK/Arabic/Spanish rules and English filler/hyphen handling.
const GOLD_TEXTS = {
  EN: [
    'Hmmm, okay.', 'Err, what?', 'Uhhuh.', 'Mmm-mmm.', 'Ah-ha!', '...umm...', '[breath] umm.', 'I said {MIS: umm}.',
    '<foreign> umm </foreign> yes.', 'Mm.', 'Um, okay.', 'Ooo, nice.', 'Aww, sweet.', 'Ooff, heavy.', 'Unh-unh.',
    'Bye-bye now.', 'Bye-Bye now.', 'It was a no-no.', 'My mother-in-law.', 'Go-go dancers.', 'A 50-50 chance.',
    'A 1-10 scale.', 'A-a-a cold.', 'Re-record it.', 'Re-re-record it.', 'Self-self.', 'I [breath]-I know.',
    'ABCDEFGHIJKLMNOPQRSTUVWXYZ letters.', 'Noooooooooooooo.', 'Call 123456789012345678901234.', 'a && b.',
    'x => y.', 'He said \\"hi\\".', '<laugh-speech> a = b </laugh-speech>', 'I {PRO: one} said.',
    'Five {PRO: 5}.', '{PRO:five} 5.', '5 {PRO : five}.', 'Unintelligible.', '[Unintelligible] yes.',
    'He said PRO: yes.', '{MIS: one} {two}', '{written} {PRO: spoken}', 'Yes [breath]. No.', 'Yes [breath]", no.',
    'Yes [breath]"word', '(Yes)', '"Yes," he said.', 'Yes!!', 'Yes?! No.', 'Yes... no.', 'Wait-- what?',
  ],
  ZH: [
    '你好【叹气】世界。', '你好 【叹气】 世界。', '【叹气】', '【叹气】。', '【 叹气 】 你好。', '【】 你好。', '【叹气 你好。',
    '你好 叹气】。', '【笑笑】 你好。', '[叹气] 你好。', '你好 【听不清】。', '听不清 你好。', '你好，世界。', '你好,世界',
    'OK, 好的。', '圆周率是3.14。', '等等...好的。', '你好；世界', '你好; 世界', '你好:世界', '时间是3:30。', '你好！？',
    '你好……！', '你好，，世界。', '你好 。', '𠀀 𠀁。', '你好 😀。', '<唱歌> 你好 </唱歌>', '<外语> hello </外语> 你好。',
    '這個 說話。', '{PRO: 五} 5。', 'ＡＢＣＤＥＦ。',
  ],
  JA: [
    '［あくび］ はい。', 'はい［あくび］いいえ。', '［ あくび ］ はい。', '［］ はい。', '［あくび はい。', '［ぬぬ］ はい。',
    '［あくび］。', '[あくび] はい。', 'はい!', 'OK!', 'はい、いいえ。', 'はい... いいえ。', 'はい……！', 'はい！？',
    'はい ［あくび］ いいえ。', 'はい、、', '<歌うように> はい </歌うように>', 'はい。 いいえ。', 'ＡＢＣＤＥ はい。',
  ],
  AR: [
    '<أجنبي="EN"> {EN} hello </أجنبي> مرحبا.', '<أجنبي> {EN}hello </أجنبي> مرحبا.', '<أجنبي>{EN} hello</أجنبي>',
    '<أجنبي> {en} hello </أجنبي> مرحبا.', 'كلمة {كلمة :PRO} لا.', 'كلمة {كلمة : PRO} لا.', 'خمسة {5 :PRO} لا.',
    'خمسة {PRO: 5} لا.', 'مرحبا، كيف حالك؟', 'ماذا؟!', 'ماذا؟؟', 'HELLO مرحبا.', '{EN}مرحبا.', 'مرحبا {XX} اليوم.',
    'MIS: كلمة.', 'غير-مفهوم هنا.', '[غير-مفهوم] هنا.',
  ],
  ES: [
    '¡¿Qué?!', '¿Qué?, dijo.', 'Hola... ¿qué?', '¡¡Hola!!', 'Sí. ¿Vienes?', '¿...?', '¿Qué?? No.', '¡Hola!, amigo.',
    'Hola, ¿qué tal?', 'Hola,, ¿qué?', 'ininteligible aquí.', '[ininteligible] aquí.',
  ],
  KO: ['발음: 하나.', '하나 {발음: 5}.', '하나 {발음: 하나}.', 'HELLO 세계.', '<웃음 섞인 발화> 네 </웃음 섞인 발화>', '네!! 아니요.', '이해할 수 없음 네.'],
  FR: ['SIC: mot.', 'Mot {MIS: mot}.', 'Mot {SIC:mot}.', 'Mot{SIC: mot}.', 'inintelligible ici.', 'Oui — non.'],
  DE: ['unverständlich hier.', '[unverständlich] hier.', '<fremdsprachig> hello </fremdsprachig> ja.', 'Ja—nein.'],
  MR: ['शब्द {उच्चार: पाच}.', 'अपभ्रंश: शब्द.', 'शब्द {अपभ्रंश: शब्द}.', 'शब्द {उच्चार: 5}.', 'हो।। नाही.'],
  HI: ['हाँ।। नहीं।', 'हाँ। नहीं।', 'HELLO दुनिया।'],
  TH: ['ครับ..', 'ครับ!!', 'ครับ?!', 'HELLO ครับ'],
  TR: ['Evet.', 'Evet!! Hayır.', 'anlaşılmayan konuşma burada.'],
  PT: ['Sim.', 'Sim -- não.'], IT: ['Sì.', 'Sì--no.'], ID: ['Ya.', 'Ya  tidak.'], TL: ['Oo.', 'Oo,, hindi.'],
  VI: ['Vâng.', 'Vâng – không.'], TE: ['అవును.', 'అవును!!'], TA: ['ஆம்.', 'ஆம்!!'], BN: ['হ্যাঁ।', 'হ্যাঁ।।'],
};

export function corpus(lintData) {
  const banned = lintData.banned_words;
  const cases = RU_TEXTS.map((text) => single('RU', text));
  cases.push(...RU_HELPER_TEXTS.map((text) => single('RU', text)));
  cases.push(single('RU', `Это ${banned.ru[0].word} слово.`, 'banned word'));
  cases.push(single('RU', `Это ${banned.ru[0].word.toUpperCase()} слово.`, 'banned word uppercase'));
  cases.push(single('RU', `Это ${banned.ru[0].word}ище слово.`, 'banned word inside a longer word'));

  cases.push(transcript('RU', 'overlap on one track', [['Первый.', 1, 3], ['Второй.', 2.5, 4]]));
  cases.push(transcript('RU', 'overlap across tracks', [['Первый.', 1, 3, 't1'], ['Второй.', 2.5, 4, 't2']]));
  cases.push(transcript('RU', 'gap under a second', [['Первый.', 1, 3], ['Второй.', 3.5, 5]]));
  cases.push(transcript('RU', 'gap of exactly a second', [['Первый.', 1, 3], ['Второй.', 4, 5]]));
  cases.push(transcript('RU', 'touching segments', [['Первый.', 1, 3], ['Второй.', 3, 5]]));
  cases.push(transcript('RU', 'three overlapping', [['Один.', 1, 5], ['Два.', 2, 6], ['Три.', 3, 7]]));
  cases.push(transcript('RU', 'unsorted segments', [['Позже.', 10, 12], ['Раньше.', 1, 3]]));
  cases.push(transcript('RU', 'exact long duplicate', [
    ['Это достаточно длинный текст для проверки повтора.', 1, 3],
    ['Между ними другой.', 5, 7],
    ['Это достаточно длинный текст для проверки повтора.', 9, 11],
  ]));
  cases.push(transcript('RU', 'adjacent mid-length duplicate', [
    ['Двадцать с лишним знаков.', 1, 3],
    ['Двадцать с лишним знаков.', 5, 7, 't2'],
  ]));
  cases.push(transcript('RU', 'short duplicate', [['Да, конечно.', 1, 3], ['Да, конечно.', 5, 7]]));
  cases.push(transcript('RU', 'foreign tag consistency', [
    ['<иностранная-речь> hello </иностранная-речь> мир.', 1, 3],
    ['Скажи hello мне.', 5, 7],
    ['Hello, мир.', 9, 11],
  ]));
  cases.push(transcript('RU', 'foreign tag consistency with code', [
    ['<иностранная-речь="EN"> hello </иностранная-речь> мир.', 1, 3],
    ['Скажи hello.', 5, 7],
  ]));
  cases.push(transcript('RU', 'foreign tag with two words', [
    ['<иностранная-речь> hello world </иностранная-речь> да.', 1, 3],
    ['Скажи hello.', 5, 7],
  ]));
  cases.push(transcript('RU', 'same speaker continuation', [
    ['Я думал...', 1, 3],
    ['и решил.', 5, 7],
    ['А ты--', 9, 11, 't2'],
    ['нет.', 13, 15, 't1'],
  ]));
  cases.push(transcript('RU', 'continuation from another speaker', [
    ['Я думал...', 1, 3, 't1'],
    ['и решил.', 5, 7, 't2'],
  ]));
  cases.push(transcript('RU', 'continuation through closers and trailing tags', [
    ['Он сказал "я думал..."', 1, 3, 't1'],
    ['Она--', 2, 4, 't2'],
    ['и решил.', 5, 7, 't1'],
    ['и она. [смех]', 6, 8, 't2'],
    ['Я-- [смех] {СКАЗ: да}', 9, 11, 't1'],
    ['нет.', 13, 15, 't1'],
  ]));
  cases.push(transcript('RU', 'only the latest same-speaker segment counts', [
    ['Я думал...', 1, 3, 't1'],
    ['Готово.', 4, 5, 't1'],
    ['и решил.', 6, 7, 't1'],
    ['Начало -', 8, 9, 't2'],
    ['и конец.', 10, 11, 't2'],
  ]));
  cases.push(transcript('RU', 'continuation with unsorted starts', [
    ['и решил.', 5, 7, 't1'],
    ['Я думал--', 1, 3, 't1'],
    ['вот. Вы пришли', 8, 9, 't1'],
  ]));

  for (const text of [
    'Uhh, okay.', 'Hm, sure.', 'Mmm yes.', 'Mm-hmm, yes.', 'Umm, I- I think so.', 'I th- think so.',
    'It was so-so.', 'She is self-aware.', 'A well-known fact.', '9-9 tie.', 'Word...word.',
    'HELLO there.', 'Ok {PRO: okay}.', 'I [laugh] know.', '[laughter] yes.', 'He said (quietly) no.',
    'I think-- no.', 'Yes—no.', 'Error 404 {MIS: four}.', 'unintelligible here.', '[unintelligible]',
    'I-I think so.', 'Th-the cat.', 'Wh-what?', 'The 9-11 report.', 'So-so-so.', 'Uh-huh.',
  ]) cases.push(single('EN', text));

  for (const text of ['مرحبا.', '<أجنبي> hello </أجنبي> مرحبا.', '<أجنبي> {EN} hello </أجنبي> مرحبا.', '<أجنبي> {XX} hello </أجنبي> مرحبا.', 'مرحبا {EN} اليوم.', 'نعم،، لا.']) {
    cases.push(single('AR', text));
  }
  for (const text of ['你好。', '這是書。', '你 好。', '你好, 世界.', '你好！！', '你好 world。', '“你好”。', '你好——世界。']) {
    cases.push(single('ZH', text));
  }
  for (const text of ['こんにちは。', 'こんにちは 世界。', 'はい、、いいえ。', 'ＡＢＣＤＥ。', 'はい。。']) {
    cases.push(single('JA', text));
  }
  for (const text of ['Hallo, Welt.', 'Ähm, ja.', 'STRASSE ist lang.']) cases.push(single('DE', text));
  for (const text of ['¿Qué? ¡No!', 'Hola,, amigo.', '¿¡Qué!?']) cases.push(single('ES', text));
  for (const text of ['Bonjour {SIC: mot}.', 'Oui -- non.']) cases.push(single('FR', text));
  for (const [language, texts] of Object.entries(GOLD_TEXTS)) {
    for (const text of texts) cases.push(single(language, text));
    const words = banned[language.toLowerCase()];
    if (words?.length) {
      cases.push(single(language, `${words[0].word} !`, 'banned word'));
      if (words[0].romanization) cases.push(single(language, `Say ${words[0].romanization}.`, 'banned romanization'));
    }
  }
  cases.push(transcript('EN', 'foreign tag consistency', [
    ['<foreign> hola </foreign> yes.', 1, 3],
    ['He said hola, Hola.', 5, 7],
    ['Holas.', 9, 11, 't2'],
  ]));
  cases.push(transcript('EN', 'foreign tag codes disagree', [
    ['<foreign="ES"> hola </foreign> yes.', 1, 3],
    ['<foreign="PT"> hola </foreign> yes.', 5, 7],
    ['He said hola.', 9, 11],
  ]));
  cases.push(transcript('ZH', 'foreign tag consistency', [
    ['<外语> hello </外语> 你好。', 1, 3],
    ['他说hello。', 5, 7],
  ]));
  cases.push(transcript('AR', 'foreign tag consistency', [
    ['<أجنبي> {EN} hello </أجنبي> مرحبا.', 1, 3],
    ['قال hello.', 5, 7],
  ]));
  cases.push(transcript('TR', 'foreign tag consistency with dotless i', [
    ['<yabancı dil> Istanbul </yabancı dil> güzel.', 1, 3],
    ['Istanbul güzel.', 5, 7],
    ['istanbul güzel.', 9, 11],
  ]));
  cases.push(transcript('DE', 'foreign tag accents and digits', [
    ['<fremdsprachig> Café2 </fremdsprachig> ja.', 1, 3],
    ['Ein cafe bitte.', 5, 7],
  ]));
  cases.push(transcript('EN', 'two tracks interleaved', [
    ['First line here.', 1, 3, 'a'],
    ['Second voice.', 2, 4, 'b'],
    ['Third line.', 3.5, 5, 'a'],
    ['Fourth voice.', 4.2, 6, 'b'],
  ]));
  cases.push(transcript('EN', 'duplicates across tracks and equal starts', [
    ['This sentence is long enough to count twice.', 1, 3, 'a'],
    ['This sentence is long enough to count twice.', 1, 2, 'b'],
    ['Twenty-two characters!', 5, 6, 'a'],
    ['Twenty-two characters!', 7, 8, 'a'],
  ]));
  const repeated = 'Привет 😀 мир, как дела у тебя сегодня вечером? ';
  cases.push(single('RU', `${repeated}${repeated}${repeated}`, 'repeated text with emoji'));
  return cases;
}
