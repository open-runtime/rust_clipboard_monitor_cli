import 'lib/src/rust/api.dart';
import 'lib/src/rust/frb_generated.dart';

void main() async {
  print('Testing silent function...');
  await RustLib.init();
  
  // Test silent function
  final data = await getCurrentClipboardInfoSilent();
  if (data != null) {
    print('✅ Silent function works!');
    print('   Change count: ${data.changeCount}');
    print('   Has window context: ${data.windowContext != null}');
    print('   Has browser context: ${data.browserContext != null}');
    print('   Has space context: ${data.spaceContext != null}');
    print('   Has accessibility context: ${data.accessibilityContext != null}');
    print('   Has system context: ${data.systemContext != null}');
  } else {
    print('❌ No clipboard data');
  }
}