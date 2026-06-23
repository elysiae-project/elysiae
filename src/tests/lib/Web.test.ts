import { expect, test } from "vitest";
import { isURLValid } from "../../lib/Web";

test('Fetch API JSON data', async() => {

});

test('Determine is URL is valid http/https URI', () => {
    expect(isURLValid("https://google.com")).toBe(true);
    expect(isURLValid("123https:aaa///1112323232323")).toBe(false);
    expect(isURLValid("http://example.com")).toBe(true);
});


test('Download a file', () => {

})