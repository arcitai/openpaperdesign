# Product page design

The page belongs to the Arc’IT family: neutral surfaces, restrained steel blue,
short copy and a clear view of the editable canvas. Keep the layout minimal:
header, purpose, source/setup actions, screenshot, upstream credit and footer.
Avoid a feature wall or installation sales panel.

## Typography and identity

Inter 400 is for the OpenPaperDesign and Arc’IT names. Geist Sans is for all
other headings, body text, navigation and controls. Supporting words such as
“An” and “project” in the footer use the reading face. Fonts and their licences
are served locally.

Use the repository-owned light/dark logo pair unchanged. `assets-sources.json`
records their exact revision and hashes. These assets come from the branding
work in PR #9; their presence does not claim its unreleased app features exist.

## Colour, composition and motion

Light: paper #fafafa, ink #202020, accent #4265a0.
Dark: paper #171717, ink #f5f5f5, accent #93b4e9.

The second headline line and primary source action use the accent. The current
release screenshot sits within a rounded grid frame with a subtle halo and
shadow. Desktop hover straightens its slight tilt. Mobile and reduced-motion
preferences disable the tilt; reduced motion also removes transitions.
There is no looping animation. Focus indicators and theme controls remain
visible, and actions wrap on narrow screens.

## Content and attribution

Keep the distinction between source code and a packaged app. The current
button downloads v0.1.0 source, and the setup guide is pinned to that release.
A future installer must be verified before changing this wording. The image
is the existing v0.1.0 screenshot until issue #19 supplies a current one.

Keep “Built on Paper RS by Rasmic (Michael Shimeles)” with the upstream links,
and the MIT/Apache links. Repository notices remain unchanged. No affiliation
with another Paper product is claimed.

Harp and paper.rasmic.xyz informed the initial exploration; no reference assets
or page code are copied. The composition and type follow Arc’IT's identity.

## Delivery boundary

See [README.md](README.md) for local serving, asset refresh and the open app,
screenshot and deployment follow-ups. Adding this source does not publish the
site or close those issues. The accepted local direction still needs final
browser verification at publication, including themes, mobile layout and links.
