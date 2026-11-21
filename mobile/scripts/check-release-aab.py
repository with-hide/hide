#!/usr/bin/env python3
# The release gate on the signed Android App Bundle: the bundle the build machine signed, unpacked, held to what the
# release is allowed to be. Prints every failed check and exits 1 when there is one; the upload never starts after a
# failure. The twin of check-release-app.sh, which holds the signed iOS app.
#
# Usage: check-release-aab.py <aab dir> <signers file> <android-release-expect.json> <bundle manifest.json> <version> <version code>
#   <aab dir>       the signed bundle, unpacked (base/manifest, base/res, base/assets, base/dex, base/lib, ...)
#   <signers file>  the SHA-256 of each certificate that signed the bundle, one per line (`keytool -printcert -jarfile`)
#   <manifest>      the bundle manifest bundle-manifest.mjs wrote: the sha256 of every file the app's public/ may hold
#
# The manifest and the XML resources are read from the bundle itself, in the compiled form the build wrote (aapt2's
# XmlNode protocol buffer), so a setting that never reached the bundle cannot pass. It needs Python's standard library
# only (the python3 macOS's command line tools carry), so its tests run wherever the mobile tests do. The permission and
# exported-component lists in android-release-expect.json are exact: a node that adds a permission or a component adds
# it there in the same change.
import hashlib
import json
import os
import sys
import xml.etree.ElementTree as ET

ANDROID = '{http://schemas.android.com/apk/res/android}'
failures = []


def fail(what):
    failures.append(what)
    print(f'release check failed: {what}', file=sys.stderr)


def check(what, holds):
    if not holds:
        fail(what)


# ---- aapt2's compiled XML ------------------------------------------------------------------------------------------
# XmlNode {element = 1, text = 2}; XmlElement {namespace_uri = 2, name = 3, attribute = 4, child = 5 (XmlNode)};
# XmlAttribute {namespace_uri = 1, name = 2, value = 3}: the value is the attribute's text as written (aapt2's "Raw").

def varint(buf, i):
    shift = result = 0
    while True:
        if i >= len(buf) or shift > 63:
            raise ValueError('a varint runs past the end')
        byte = buf[i]
        i += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, i
        shift += 7


def fields(buf):
    i = 0
    while i < len(buf):
        key, i = varint(buf, i)
        number, wire = key >> 3, key & 7
        if wire == 0:
            value, i = varint(buf, i)
        elif wire == 2:
            size, i = varint(buf, i)
            if i + size > len(buf):
                raise ValueError('a field runs past the end')
            value, i = buf[i:i + size], i + size
        elif wire in (1, 5):
            width = 8 if wire == 1 else 4
            value, i = buf[i:i + width], i + width
        else:
            raise ValueError(f'unknown wire type {wire}')
        yield number, wire, value


def qualified(namespace, name):
    return '{%s}%s' % (namespace, name) if namespace else name


def element(buf):
    namespace = name = ''
    attributes, children = [], []
    for number, wire, value in fields(buf):
        if wire != 2:
            continue
        if number == 2:
            namespace = value.decode()
        elif number == 3:
            name = value.decode()
        elif number == 4:
            attribute = {n: v.decode() for n, w, v in fields(value) if w == 2 and n in (1, 2, 3)}
            attributes.append((qualified(attribute.get(1, ''), attribute.get(2, '')), attribute.get(3, '')))
        elif number == 5:
            children.extend(v for n, w, v in fields(value) if n == 1 and w == 2)
    node = ET.Element(qualified(namespace, name))
    for key, value in attributes:
        node.set(key, value)
    for child in children:
        node.append(element(child))
    return node


def read_xml(path):
    try:
        with open(path, 'rb') as stream:
            roots = [value for number, wire, value in fields(stream.read()) if number == 1 and wire == 2]
        if len(roots) != 1:
            raise ValueError('it holds no single root element')
        return element(roots[0])
    except (OSError, ValueError, UnicodeDecodeError) as error:
        fail(f'{path} cannot be read as compiled XML: {error}')
        return None


# ---- the rest ------------------------------------------------------------------------------------------------------

def attr(node, name):
    return None if node is None else node.get(ANDROID + name)


def is_true(value):
    return value is not None and value.strip().lower() in ('true', '1')


def files_under(root):
    found = []
    for folder, _, names in os.walk(root):
        found.extend(os.path.relpath(os.path.join(folder, name), root).replace(os.sep, '/') for name in names)
    return sorted(found)


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def xml_resource(aab, application, attribute):
    """The res/xml file the application names (`@xml/<name>`), alone: a qualified copy would replace it on some phones."""
    reference = attr(application, attribute)
    if not reference or not reference.startswith('@xml/'):
        fail(f'the app names no @xml/ resource as android:{attribute}')
        return None
    name = reference[len('@xml/'):] + '.xml'
    res = os.path.join(aab, 'base/res')
    others = [folder for folder in sorted(os.listdir(res)) if folder.startswith('xml-') and os.path.isfile(os.path.join(res, folder, name))]
    check(f'{name} has no qualified copies ({", ".join(others)})', not others)
    return read_xml(os.path.join(res, 'xml', name))


def main(argv):
    if len(argv) != 7:
        print('usage: check-release-aab.py <aab dir> <signers file> <expect.json> <manifest.json> <version> <version code>', file=sys.stderr)
        return 2
    aab, signers_path, expect_path, manifest_path, version, version_code = argv[1:]
    for path in [os.path.join(aab, 'base/manifest/AndroidManifest.xml'), os.path.join(aab, 'base/assets/capacitor.config.json'),
                 signers_path, expect_path, manifest_path]:
        if not os.path.isfile(path) or os.path.getsize(path) == 0:
            print(f'release check failed: {path} is missing or empty', file=sys.stderr)
            return 1
    with open(expect_path) as stream:
        want = json.load(stream)
    with open(manifest_path) as stream:
        bundle_manifest = json.load(stream)

    manifest = read_xml(os.path.join(aab, 'base/manifest/AndroidManifest.xml'))
    if manifest is None:
        return 1
    application = manifest.find('application')
    sdk = manifest.find('uses-sdk')

    # The identity of the build.
    check(f'package is {want["package"]}', manifest.get('package') == want['package'])
    check(f'version is {version}', attr(manifest, 'versionName') == version)
    check(f'version code is {version_code}', attr(manifest, 'versionCode') == version_code)
    check(f'minimum SDK is {want["minSdk"]}', attr(sdk, 'minSdkVersion') == str(want['minSdk']))
    check(f'target SDK is {want["targetSdk"]}', attr(sdk, 'targetSdkVersion') == str(want['targetSdk']))
    modules = sorted(name for name in os.listdir(aab) if os.path.isdir(os.path.join(aab, name)) and name not in ('META-INF', 'BUNDLE-METADATA'))
    check(f'the bundle has the base module only (modules: {", ".join(modules)})', modules == ['base'])
    if application is None:
        fail('the manifest has no application element')
        return finish(version, version_code)

    # No debug left in.
    check('the app is not debuggable', not is_true(attr(application, 'debuggable')))
    check('the app is not marked test-only', not is_true(attr(application, 'testOnly')))

    # Nothing leaves the phone in clear text: no manifest switch, and a network security configuration whose every
    # entry refuses it.
    check('the manifest allows no cleartext traffic', not is_true(attr(application, 'usesCleartextTraffic')))
    security = xml_resource(aab, application, 'networkSecurityConfig')
    if security is not None:
        base = security.find('base-config')
        check('the network security configuration refuses cleartext by default', base is not None and base.get('cleartextTrafficPermitted') == 'false')
        permits = [node.tag for node in security.iter() if node.get('cleartextTrafficPermitted') not in (None, 'false')]
        check(f'no part of the network security configuration permits cleartext ({", ".join(permits)})', not permits)
        check('the network security configuration has no debug overrides', security.find('debug-overrides') is None)
        check('the network security configuration trusts no user certificates', not any(node.get('src') == 'user' for node in security.iter('certificates')))

    # Nothing the app keeps is backed up or moved to another phone.
    check('backup is off', attr(application, 'allowBackup') == 'false')
    rules = xml_resource(aab, application, 'dataExtractionRules')
    if rules is not None:
        for section in ('cloud-backup', 'device-transfer'):
            part = rules.find(section)
            excluded = set() if part is None else {node.get('domain') for node in part.findall('exclude') if node.get('path') == '.'}
            missing = sorted(set(want['excludedDomains']) - excluded)
            check(f'{section} excludes every domain (missing: {", ".join(missing) or "the section"})', part is not None and not missing)
            check(f'{section} includes nothing', part is None or part.find('include') is None)

    # The exact permission lists: what the app asks for and what it declares.
    asked = sorted(attr(node, 'name') or '' for tag in ('uses-permission', 'uses-permission-sdk-23') for node in manifest.findall(tag))
    check(f'the app asks for exactly the permissions android-release-expect.json lists (asks: {", ".join(asked)})', asked == sorted(want['permissions']))
    declared = sorted(attr(node, 'name') or '' for node in manifest.findall('permission'))
    check(f'the app declares exactly the permissions android-release-expect.json lists (declares: {", ".join(declared)})', declared == sorted(want['declaredPermissions']))

    # The exact list of components another app can start: exported explicitly, or by an intent filter with no exported
    # attribute (which Android 12 refuses to install, but a library's manifest could still carry).
    exported = []
    for node in application:
        if node.tag in ('activity', 'activity-alias', 'service', 'receiver', 'provider'):
            flag = attr(node, 'exported')
            if is_true(flag) or (flag is None and node.find('intent-filter') is not None):
                exported.append({'kind': node.tag, 'name': attr(node, 'name'), 'permission': attr(node, 'permission')})
    key = lambda entry: (entry['kind'], entry['name'] or '')
    found = sorted(exported, key=key)
    check(f'the exported components are exactly the list in android-release-expect.json (exported: {json.dumps(found)})', found == sorted(want['exported'], key=key))

    # The web view: no inspection, no logging, no remote page, no navigation, no cleartext.
    with open(os.path.join(aab, 'base/assets/capacitor.config.json')) as stream:
        config = json.load(stream)
    server = config.get('server') or {}
    android = config.get('android') or {}
    check('the web view cannot be inspected', android.get('webContentsDebuggingEnabled', False) is False)
    check('Capacitor logging is none', config.get('loggingBehavior') == 'none')
    check('the app loads no server url', 'url' not in server and server.get('cleartext') is not True)
    check('the app allows no navigation', (server.get('allowNavigation') or []) == [])
    check('Capacitor allows no mixed content', android.get('allowMixedContent', False) is False)

    # The code: the debug build's inputs are compiled out.
    dex_root = os.path.join(aab, 'base/dex')
    dex = [path for path in files_under(dex_root) if path.endswith('.dex')] if os.path.isdir(dex_root) else []
    check('the bundle holds code', bool(dex))
    for path in dex:
        with open(os.path.join(dex_root, path), 'rb') as stream:
            body = stream.read()
        for word in want['forbiddenDexStrings']:
            check(f'{path} holds no debug hook ({word})', word.encode() not in body)

    # No native library the app does not ship on purpose.
    lib_root = os.path.join(aab, 'base/lib')
    libraries = files_under(lib_root) if os.path.isdir(lib_root) else []
    check(f'the native libraries are exactly the list in android-release-expect.json (found: {", ".join(libraries) or "none"})', libraries == sorted(want['nativeLibraries']))

    # The web bundle: every file in assets/public/ is in the manifest, with its digest, and none is missing.
    public = os.path.join(aab, 'base/assets/public')
    if os.path.isdir(public):
        actual = files_under(public)
        wanted = sorted(bundle_manifest['files'])
        differ = len(set(actual) ^ set(wanted))
        check(f'assets/public/ holds different files than the bundle manifest ({differ} differ)', differ == 0)
        for path in wanted:
            full = os.path.join(public, path)
            if os.path.isfile(full):
                check(f'assets/public/{path} differs from the bundle manifest', sha256_of(full) == bundle_manifest['files'][path])
    else:
        fail('the bundle has no assets/public/ folder')

    # Signed with the upload key, and only it.
    with open(signers_path) as stream:
        signers = sorted({line.strip().replace(':', '').upper() for line in stream if line.strip()})
    check(f'the bundle is signed by the upload key alone (signers: {", ".join(signers)})', signers == [want['uploadCertificateSha256'].replace(':', '').upper()])
    return finish(version, version_code)


def finish(version, version_code):
    if failures:
        print(f'{len(failures)} release check(s) failed', file=sys.stderr)
        return 1
    print(f'release checks passed for {version} ({version_code})')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
