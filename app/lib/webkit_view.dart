// Android, iOS, macOS: the system webview (webview_flutter).
//
// These webviews ignore downloads, and the macOS one ignores file inputs, so
// the web client talks to the app through the `ArusApp` JavaScript channel
// (see crates/arrus-web/src/web.rs):
//
//   page -> app  {"type": "save", "name", "mime", "data": base64}
//                  -> a native save dialog (share / "Save to Files" on phones)
//   page -> app  {"type": "pick", "id", "accept", "multiple"}   (macOS only)
//   app -> page  window.arusAppPicked(id, [{"name", "type", "data": base64}])
//
// The page knows to ask for picks because the app sets
// `window.arusAppPicks = true` (macOS) after each page load.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

import 'common.dart';

class WebKitView extends StatefulWidget {
  const WebKitView({super.key, required this.url});

  final String url;

  @override
  State<WebKitView> createState() => _WebKitViewState();
}

class _WebKitViewState extends State<WebKitView> {
  late final WebViewController _controller;
  bool _loaded = false;
  bool _failed = false;
  String _error = '';

  Uri get _home => Uri.parse(widget.url);

  @override
  void initState() {
    super.initState();
    final PlatformWebViewControllerCreationParams params;
    if (WebViewPlatform.instance is WebKitWebViewPlatform) {
      params = WebKitWebViewControllerCreationParams(
        allowsInlineMediaPlayback: true,
        // Music starts from the play button, not on its own; no extra gesture needed.
        mediaTypesRequiringUserAction: const <PlaybackMediaTypes>{},
      );
    } else {
      params = const PlatformWebViewControllerCreationParams();
    }
    final c = WebViewController.fromPlatformCreationParams(params);
    c
      ..setJavaScriptMode(JavaScriptMode.unrestricted)
      ..addJavaScriptChannel('ArusApp', onMessageReceived: _onMessage)
      ..setNavigationDelegate(NavigationDelegate(
        onPageFinished: (_) {
          if (Platform.isMacOS) {
            c.runJavaScript('window.arusAppPicks = true;');
          }
          if (!_loaded && !_failed) setState(() => _loaded = true);
        },
        onWebResourceError: (e) {
          if (e.isForMainFrame ?? true) {
            setState(() {
              _failed = true;
              _error = e.description;
            });
          }
        },
        onNavigationRequest: _onNavigation,
      ));
    // The page is dark: no white flash while loading (iOS/Android; macOS
    // webviews have no background colour).
    if (!Platform.isMacOS) {
      c.setBackgroundColor(background);
    }
    final platform = c.platform;
    if (platform is AndroidWebViewController) {
      platform
        ..setMediaPlaybackRequiresUserGesture(false)
        ..setOnShowFileSelector(_androidFiles);
    }
    if (platform is WebKitWebViewController && Platform.isIOS) {
      platform.setAllowsBackForwardNavigationGestures(true);
    }
    _controller = c;
    c.loadRequest(_home);
  }

  /// Links to other sites open in the browser.
  Future<NavigationDecision> _onNavigation(NavigationRequest r) async {
    final uri = Uri.tryParse(r.url);
    if (uri == null || !r.isMainFrame) return NavigationDecision.navigate;
    final web = uri.scheme == 'http' || uri.scheme == 'https';
    if (web && uri.host == _home.host) return NavigationDecision.navigate;
    if (uri.scheme == 'about' || uri.scheme == 'data' || uri.scheme == 'blob') {
      return NavigationDecision.navigate;
    }
    await launchUrl(uri, mode: LaunchMode.externalApplication);
    return NavigationDecision.prevent;
  }

  FileType _typeFor(String accept) {
    if (accept.contains('audio')) return FileType.audio;
    if (accept.startsWith('image/') && !accept.contains('.')) return FileType.image;
    return FileType.any;
  }

  /// Android's webview asks the app for <input type=file>; it wants content URIs.
  Future<List<String>> _androidFiles(FileSelectorParams p) async {
    final type = _typeFor(p.acceptTypes.join(','));
    final files = p.mode == FileSelectorMode.openMultiple
        ? await FilePicker.pickFiles(type: type)
        : [?await FilePicker.pickFile(type: type)];
    return files.map((f) => f.uri.toString()).toList();
  }

  Future<void> _onMessage(JavaScriptMessage m) async {
    final Map<String, dynamic> msg;
    try {
      msg = jsonDecode(m.message) as Map<String, dynamic>;
    } catch (_) {
      return;
    }
    switch (msg['type']) {
      case 'save':
        await _save(msg['name'] as String? ?? 'arus', msg['mime'] as String? ?? 'application/octet-stream',
            msg['data'] as String? ?? '');
      case 'pick':
        await _pick((msg['id'] as num?)?.toInt() ?? 0, msg['accept'] as String? ?? '', msg['multiple'] == true);
    }
  }

  Future<void> _save(String name, String mime, String data) async {
    try {
      final saved = await FilePicker.saveFile(fileName: name, bytes: base64Decode(data), mimeType: mime);
      if (saved != null) _toast('Saved $name');
    } catch (e) {
      _toast('Could not save $name: $e');
    }
  }

  Future<void> _pick(int id, String accept, bool multiple) async {
    final out = <Map<String, String>>[];
    try {
      final type = _typeFor(accept);
      final files = multiple ? await FilePicker.pickFiles(type: type) : [?await FilePicker.pickFile(type: type)];
      for (final f in files) {
        out.add({'name': f.name, 'type': f.xFile.mimeType ?? '', 'data': base64Encode(await f.xFile.readAsBytes())});
      }
    } catch (e) {
      _toast('Could not open the file: $e');
    }
    await _controller.runJavaScript('window.arusAppPicked && window.arusAppPicked($id, ${jsonEncode(out)});');
  }

  void _toast(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  void _retry() {
    setState(() {
      _failed = false;
      _loaded = false;
    });
    _controller.loadRequest(_home);
  }

  /// Android's back button goes back in the page first.
  Future<void> _back(bool didPop, Object? _) async {
    if (didPop) return;
    if (await _controller.canGoBack()) {
      await _controller.goBack();
    } else {
      await SystemNavigator.pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    return AnnotatedRegion<SystemUiOverlayStyle>(
      value: SystemUiOverlayStyle.light.copyWith(
        statusBarColor: background,
        systemNavigationBarColor: background,
      ),
      child: PopScope(
        canPop: false,
        onPopInvokedWithResult: _back,
        child: Scaffold(
          body: SafeArea(
            child: Stack(
              children: [
                Positioned.fill(child: WebViewWidget(controller: _controller)),
                if (!_loaded || _failed)
                  Positioned.fill(
                    child: overlay(failed: _failed, url: widget.url, error: _error, retry: _retry),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
