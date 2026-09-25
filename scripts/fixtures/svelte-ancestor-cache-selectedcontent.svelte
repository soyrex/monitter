<select>
  <selectedcontent><span class="selected-value">Current choice</span></selectedcontent>
  <option value="one"><span class="option-value">One</span></option>
  <option value="two"><span class="option-value">Two</span></option>
</select>

<style>
  select > selectedcontent > .selected-value { color: red; }
  select > option > .option-value { color: blue; }
  select:has(option:checked) .selected-value { font-weight: 600; }
</style>
