import { mount, signal, cleanup, View, Text, Input } from "nsh";
import { nativeComponent } from "nsh/native";
const Probe = nativeComponent<{value: string, beat: number}, {}>("monitter.probe", { props: ["value", "beat"], events: [] });
function App() {
 const count = signal(0);
 const text = signal("Type here");
 const network = signal("pending");
 fetch("http://127.0.0.1:9/nsh-capability-probe")
   .then(() => network.set("FAIL:allowed"))
   .catch(error => network.set(String(error)));
 const beat = signal(0);
 const timer = setInterval(() => beat.update(n => n + 1), 250);
 cleanup(() => clearInterval(timer));
 return <View style={{flex: 1, padding: 24, gap: 16, backgroundColor: "#202936"}}>
   <Text style={{fontSize: 22, color: "#ffffff"}}>NSH native surface</Text>
   <Text style={{fontSize: 18, color: "#ffffff"}}>Count: {count()}</Text>
   <View onPress={() => count.set(count() + 1)} style={{height: 44, padding: 10, backgroundColor: "#355785"}}><Text style={{color: "#ffffff"}}>Increment NSH</Text></View>
   <Input value={text()} onChange={(t) => text.set(t)} style={{height: 40, backgroundColor: "#ffffff", color: "#111111"}} />
   <Text style={{fontSize: 16, color: "#ffffff"}}>Echo: {text()}</Text>
   <Text style={{fontSize: 12, color: "#ffffff"}}>Network: {network()}</Text>
   <Probe value={`${count()}:${text()}|network=${network()}`} beat={beat()} style={{height: 40}} />
 </View>;
}
mount(App);
