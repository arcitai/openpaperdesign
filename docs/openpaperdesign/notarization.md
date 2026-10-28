# Apple signing and notarization

The current OpenPaperDesign Mac preview is **ad-hoc signed, not Apple notarized**.
The universal archive has Intel and Apple Silicon executables; that is separate from
Apple trusting the app on a newly downloaded first launch. Do not label the website
“notarized” until the final distributed archive has passed the checks below.

## What is needed from the owner

- An active Apple Developer Program membership (Apple currently lists US$99 per year,
  or the local equivalent).
- A **Developer ID Application** signing identity and its private key in the build
  Mac's Keychain. An Apple Development identity or an ad-hoc signature is insufficient.
- A notarytool Keychain profile, created by the owner using Apple's supported
  authentication. Keep passwords, private keys and API keys out of the repository,
  logs, screenshots and command history. The release workflow currently has no signing
  secrets and must continue to build honest ad-hoc previews without them.

On 28 October 2026 this machine reported **zero valid code-signing identities**.
That says nothing about whether the owner has an Apple Developer account. It prevents
completing the actual Developer ID signing and submission here today.

## Release procedure once those prerequisites exist

Use a clean, reviewed universal build from the existing Desktop packages workflow.
Keep the original CI archive and its checksum. Work on a copy in a new local temporary
directory outside synced Documents; some file providers add Finder metadata that
invalidates signatures. Never remove quarantine or weaken Gatekeeper to qualify a build.

1. Verify the original archive's SHA256 and `Contents/Resources/BUILD.json` revision,
   component revisions, platform and architectures. Extract with `ditto`. Check both
   `Contents/MacOS/openpaperdesign` and `Contents/MacOS/paper-rs-mcp` with
   `lipo -verify_arch x86_64 arm64`. Preserve the bundled notices and resources.
2. If signing metadata in BUILD.json needs updating, do it **before** signing. Record
   the original archive hash and new signing process in an external release receipt.
   Do not change application contents after notarization; doing so invalidates the seal.
3. Sign the inner bridge executable, then the outer app with the selected Developer ID
   Application identity, a secure timestamp and Hardened Runtime. Check any newly added
   nested executables/frameworks inside out as well. Do not use `--deep` as a substitute
   for signing nested code. There is currently no need for debugging entitlements such
   as `get-task-allow`.

   ```sh
   codesign --force --options runtime --timestamp --sign "$OPD_SIGNING_IDENTITY" \
     "$OPD_APP/Contents/MacOS/paper-rs-mcp"
   codesign --force --options runtime --timestamp --sign "$OPD_SIGNING_IDENTITY" \
     "$OPD_APP"
   codesign --verify --deep --strict --verbose=2 "$OPD_APP"
   ```

4. Create a fresh ZIP of the signed app with `ditto -c -k --keepParent`. Submit it:

   ```sh
   xcrun notarytool submit "$OPD_SUBMISSION_ZIP" \
     --keychain-profile "$OPD_NOTARY_PROFILE" --wait --output-format json
   ```

   Require **Accepted**, save the submission ID, and inspect Apple's log on any other
   outcome. Signing alone, or a successfully uploaded submission, is not acceptance.
5. Attach Apple's ticket to the app and verify it:

   ```sh
   xcrun stapler staple "$OPD_APP"
   xcrun stapler validate "$OPD_APP"
   codesign --verify --deep --strict --verbose=2 "$OPD_APP"
   spctl --assess --type execute --verbose=2 "$OPD_APP"
   ```

   ZIP files cannot themselves be stapled. Recreate the final ZIP **after** stapling
   the app. Compute a new SHA256; do not reuse the CI archive's checksum.
6. Extract that exact final ZIP into a second clean directory. Re-run signature,
   ticket and Gatekeeper assessments there. Test a real browser download on Mac,
   including first launch, Home, editing, save, quit/reopen and optional local MCP.
   Keep Intel runtime testing distinct from checking universal Mach-O slices on an
   Apple Silicon Mac. Do not execute unrelated third-party code as part of this test.
7. Publish only the qualified final archive and matching checksum, together with its
   original source/component provenance, signing identity/team, submission receipt
   and actual platform test results. Update the release manifest and install guide
   to match that release. Do not silently replace a published artifact. The current
   draft-release workflow and site deployment are separate steps.

The commands above are a runbook; they have **not** been submitted using an owner
identity. CI signing automation should be added only once the owner chooses how its
certificate/profile is managed. No Apple enrollment, payment, new credential, updater
or expanded secret access has been created by this work.

## Reference check

Rasmic's current Sketch v0.1.0 Mac ZIP was inspected on 28 October 2026 without launching
it. It has a Developer ID Application signature, Hardened Runtime and a stapled ticket;
`stapler validate` passed and Gatekeeper reported `source=Notarized Developer ID`.
Its release script signs nested code, submits to notarytool and staples accepted results.
This is useful packaging guidance, not an identity or credential to reuse.

- [Apple: notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Apple: customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Apple: resolving notarization issues](https://developer.apple.com/documentation/security/resolving-common-notarization-issues)
- [Apple Developer Program](https://developer.apple.com/programs/)
- [Sketch v0.1.0 release](https://github.com/michaelshimeles/sketch/releases/tag/v0.1.0)
- [Sketch Mac release script](https://github.com/michaelshimeles/sketch/blob/main/scripts/release-macos.sh)
